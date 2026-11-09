//! Whether code belongs to the project: it lies inside the project directory and no
//! `.gitignore` between that directory and the code excludes it. The project directory of a file
//! is the root of the nearest manifest of its language ([`project_root`]), so a sibling project
//! below the same parent directory is not part of it.
//!
//! This is what makes a package a dependency without naming it: the cargo registry, a virtual
//! environment and `node_modules` are outside the directory or ignored.

use std::path::{Component, Path, PathBuf};

use ignore::Match;
use ignore::gitignore::Gitignore;

use super::{Origin, Syntax};

/// The origin of the package `name` whose files are at `path`: the project when they are inside
/// `boundary` and not ignored, otherwise a dependency.
pub fn locate(path: &Path, boundary: &Path, name: &str) -> Origin {
    let path = clean(path);
    let boundary = clean(boundary);
    if path.starts_with(&boundary) && !ignored(&path, &boundary) {
        Origin::Project
    } else {
        Origin::Dependency(name.to_string())
    }
}

/// Whether a `.gitignore` in `boundary`, or in a folder below it down to the folder of `path`,
/// ignores `path`. A deeper file overrides a shallower one, as in git.
fn ignored(path: &Path, boundary: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(boundary) else {
        return false;
    };
    let is_dir = path.is_dir();
    let mut folder = boundary.to_path_buf();
    let mut folders = vec![folder.clone()];
    let parents = rel.components().count().saturating_sub(1);
    for component in rel.components().take(parents) {
        folder.push(component);
        folders.push(folder.clone());
    }
    let mut verdict = false;
    for folder in folders {
        let (gitignore, _) = Gitignore::new(folder.join(".gitignore"));
        if gitignore.is_empty() {
            continue;
        }
        match gitignore.matched_path_or_any_parents(path, is_dir) {
            Match::Ignore(_) => verdict = true,
            Match::Whitelist(_) => verdict = false,
            Match::None => {}
        }
    }
    verdict
}

/// The project that the code in `file` belongs to: the nearest folder at or above it that holds a
/// manifest of the language of `syntax`, or the outermost folder above that declares a workspace
/// (see [`Packages::declares_workspace`](super::Packages::declares_workspace)), so the members of
/// one workspace are one project. Folders above `search` are not looked at, and without a manifest
/// the project is `search`, the directory that file names are relative to. Projects that sit side
/// by side below `search` are different projects, each a dependency of the other.
pub fn project_root(syntax: &dyn Syntax, file: &Path, search: &Path) -> PathBuf {
    let search = clean(search);
    let mut near = None;
    let mut workspace = None;
    let mut dir = clean(file).parent().map(Path::to_path_buf);
    while let Some(current) = dir {
        if !current.starts_with(&search) {
            break;
        }
        if has_manifest(syntax, &current) {
            if near.is_none() {
                near = Some(current.clone());
            }
            if syntax.declares_workspace(&current) {
                workspace = Some(current.clone());
            }
        }
        if current == search {
            break;
        }
        dir = current.parent().map(Path::to_path_buf);
    }
    workspace.or(near).unwrap_or(search)
}

fn has_manifest(syntax: &dyn Syntax, dir: &Path) -> bool {
    syntax
        .manifests()
        .iter()
        .any(|name| dir.join(name).is_file())
}

/// `path` without `.` and `..` components, without touching the disk.
pub(crate) fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}
