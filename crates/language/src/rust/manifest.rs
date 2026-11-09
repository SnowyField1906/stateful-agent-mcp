//! Cargo manifests: which names a crate depends on, and where its path dependencies live.

use std::fs;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

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
