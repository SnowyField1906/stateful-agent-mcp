//! Python packages: `pyproject.toml` marks a project, and the interpreter knows its standard
//! library.

use std::path::{Path, PathBuf};

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
