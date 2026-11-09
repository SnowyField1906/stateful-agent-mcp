//! Cargo manifests: which names a crate depends on, and where its path dependencies live.

use std::fs;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::project::clean;
use crate::tables::{read, section};

/// How a manifest declares a dependency.
pub enum Declared {
    /// `name = { path = ".." }`, resolved against the manifest's directory.
    Path(PathBuf),
    /// A registry, git or other dependency without a local path.
    Package,
}

/// The dependency that code names `name` (`serde-json` is `serde_json` in code), declared by the
/// nearest manifest at or above the folder of `from`, searching up to `boundary`. A dependency
/// inherited from the workspace is looked up in `[workspace.dependencies]` of a manifest above.
pub fn find(name: &str, from: &Path, boundary: &Path) -> Option<Declared> {
    let want = normalize(name);
    let mut inherited = false;
    let mut dir = from.parent();
    while let Some(current) = dir {
        if let Some(manifest) = read(&current.join("Cargo.toml")) {
            let tables: Vec<&Table> = if inherited {
                section(&manifest, &["workspace", "dependencies"])
                    .into_iter()
                    .collect()
            } else {
                ["dependencies", "dev-dependencies", "build-dependencies"]
                    .iter()
                    .filter_map(|key| section(&manifest, &[*key]))
                    .collect()
            };
            for table in tables {
                let Some((_, value)) = table.iter().find(|(key, _)| normalize(key) == want) else {
                    continue;
                };
                match declared(value, current) {
                    Some(found) => return Some(found),
                    None => inherited = true,
                }
            }
        }
        if current == boundary {
            break;
        }
        dir = current.parent();
    }
    None
}

/// The crate roots of the package in `dir`: its library (`[lib] path`, else `src/lib.rs`), its
/// binaries (`src/main.rs`, `src/bin/*.rs`, `src/bin/*/main.rs`, `[[bin]] path`). Only the ones
/// that exist, sorted.
pub fn entrypoints(dir: &Path) -> Vec<PathBuf> {
    let manifest = read(&dir.join("Cargo.toml"));
    let src = dir.join("src");
    let declared = |key: &str| -> Option<PathBuf> {
        let path = manifest.as_ref()?.get(key)?.get("path")?.as_str()?;
        Some(dir.join(path))
    };
    let mut found = vec![declared("lib").unwrap_or_else(|| src.join("lib.rs"))];
    found.push(src.join("main.rs"));
    let bins = manifest.as_ref().and_then(|m| m.get("bin"));
    for bin in bins.and_then(Value::as_array).into_iter().flatten() {
        if let Some(path) = bin.get("path").and_then(Value::as_str) {
            found.push(dir.join(path));
        }
    }
    for entry in fs::read_dir(src.join("bin"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            found.push(path.join("main.rs"));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
    found.retain(|path| path.is_file());
    found.sort();
    found.dedup();
    found
}

/// The folders of the members of the workspace in `dir`: the ones `[workspace] members` lists (a
/// trailing `/*` stands for every folder below it that holds a manifest) and the root package.
/// Empty when the manifest in `dir` declares no workspace.
pub fn members(dir: &Path) -> Vec<PathBuf> {
    let Some(root) = read(&dir.join("Cargo.toml")) else {
        return Vec::new();
    };
    let Some(workspace) = section(&root, &["workspace"]) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let listed = workspace.get("members").and_then(Value::as_array);
    for entry in listed.into_iter().flatten().filter_map(Value::as_str) {
        match entry.strip_suffix("/*") {
            Some(parent) => {
                for child in fs::read_dir(dir.join(parent))
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    found.push(clean(&child.path()));
                }
            }
            None if !entry.contains(['*', '?', '[']) => found.push(clean(&dir.join(entry))),
            None => {}
        }
    }
    if root.contains_key("package") {
        found.push(clean(dir));
    }
    found.retain(|member| member.join("Cargo.toml").is_file());
    found.sort();
    found.dedup();
    found
}

/// The path dependencies between the members of the workspace in `dir`, as (member folder, folder
/// of the member it depends on). A dependency inherited with `workspace = true` is looked up in
/// `[workspace.dependencies]`.
pub fn member_links(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let members = members(dir);
    if members.is_empty() {
        return Vec::new();
    }
    let root = read(&dir.join("Cargo.toml"));
    let mut shared: Vec<(String, PathBuf)> = Vec::new();
    if let Some(table) = root
        .as_ref()
        .and_then(|root| section(root, &["workspace", "dependencies"]))
    {
        for (name, value) in table.iter() {
            if let Some(path) = value.get("path").and_then(Value::as_str) {
                shared.push((normalize(name), clean(&dir.join(path))));
            }
        }
    }

    let mut links = Vec::new();
    for member in &members {
        let Some(manifest) = read(&member.join("Cargo.toml")) else {
            continue;
        };
        for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
            let Some(table) = section(&manifest, &[key]) else {
                continue;
            };
            for (name, value) in table.iter() {
                let target = match value.get("path").and_then(Value::as_str) {
                    Some(path) => Some(clean(&member.join(path))),
                    None if value.get("workspace").and_then(Value::as_bool) == Some(true) => {
                        let want = normalize(name);
                        shared
                            .iter()
                            .find(|(shared_name, _)| *shared_name == want)
                            .map(|(_, path)| path.clone())
                    }
                    None => None,
                };
                if let Some(target) = target
                    && target != *member
                    && members.contains(&target)
                {
                    links.push((member.clone(), target));
                }
            }
        }
    }
    links.sort();
    links.dedup();
    links
}

/// `None` when the dependency is inherited from the workspace.
fn declared(value: &Value, dir: &Path) -> Option<Declared> {
    let Value::Table(table) = value else {
        return Some(Declared::Package);
    };
    if let Some(path) = table.get("path").and_then(Value::as_str) {
        return Some(Declared::Path(dir.join(path)));
    }
    if table.get("workspace").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    Some(Declared::Package)
}

fn normalize(name: &str) -> String {
    name.replace('-', "_")
}
