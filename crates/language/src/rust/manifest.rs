//! Cargo manifests: which names a crate depends on, and where its path dependencies live.

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
