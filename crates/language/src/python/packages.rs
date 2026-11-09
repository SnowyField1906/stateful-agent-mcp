//! Python packages: `pyproject.toml` marks a project, and the interpreter knows its standard
//! library.

use std::path::{Path, PathBuf};

use toml::Value;

use crate::{Origin, Package, Packages, locate, tables};

use super::PythonSyntax;
use super::stdlib;

impl Packages for PythonSyntax {
    fn manifests(&self) -> &'static [&'static str] {
        &["pyproject.toml"]
    }

    fn declares_workspace(&self, dir: &Path) -> bool {
        tables::read(&dir.join("pyproject.toml")).is_some_and(|manifest| {
            tables::section(&manifest, &["tool", "uv", "workspace"]).is_some()
        })
    }

    fn package_origin(&self, package: &Package<'_>) -> Origin {
        let root = package.root;
        if package.relative {
            return Origin::Project;
        }
        if let Some(found) = project_package(root, package.from, package.project) {
            return locate(&found, package.project, root);
        }
        // The interpreter knows which modules are the standard library; any other module that the
        // project does not contain is installed.
        match stdlib::modules() {
            Some(modules) if modules.contains(root) => Origin::Std,
            Some(_) => Origin::Dependency(root.to_string()),
            None => Origin::Unknown,
        }
    }

    fn entrypoints(&self, dir: &Path) -> Vec<PathBuf> {
        entrypoints(dir)
    }
}

/// The modules that `[project.scripts]` and `[project.gui-scripts]` name, and the `__main__.py` of
/// the project folder, of its `src/` and of the packages directly in them.
fn entrypoints(dir: &Path) -> Vec<PathBuf> {
    let roots = [dir.to_path_buf(), dir.join("src")];
    let mut found = Vec::new();
    if let Some(manifest) = tables::read(&dir.join("pyproject.toml")) {
        for key in ["scripts", "gui-scripts"] {
            let Some(scripts) = tables::section(&manifest, &["project", key]) else {
                continue;
            };
            for target in scripts.values().filter_map(Value::as_str) {
                // `package.module:function`
                let module = target
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .replace('.', "/");
                if module.is_empty() {
                    continue;
                }
                for base in &roots {
                    found.push(base.join(format!("{module}.py")));
                    found.push(base.join(&module).join("__init__.py"));
                }
            }
        }
    }
    for base in &roots {
        found.push(base.join("__main__.py"));
        for entry in std::fs::read_dir(base).into_iter().flatten().flatten() {
            found.push(entry.path().join("__main__.py"));
        }
    }
    found.retain(|path| path.is_file());
    found.sort();
    found.dedup();
    found
}

/// The module or package folder named `root` in a source folder that holds `from` or lies above
/// it: Python finds absolute imports in the folder of the script and in the source roots of the
/// project, which is `src/` or the project folder.
fn project_package(root: &str, from: &Path, boundary: &Path) -> Option<PathBuf> {
    let mut dir = from.parent();
    while let Some(current) = dir {
        for base in [current.to_path_buf(), current.join("src")] {
            for candidate in [
                base.join(root),
                base.join(format!("{root}.py")),
                base.join(format!("{root}.pyi")),
            ] {
                if candidate.exists() {
                    return Some(candidate);
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
