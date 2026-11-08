//! Project-relative path validation for write operations, and path filters.

use std::fs::{self, Metadata};
use std::path::{Component, Path};

use super::{AgentError, Result};

/// The folder under the project root that holds the ledger, instructions and configuration.
pub const AGENTS_DIR: &str = ".AGENTS";

/// Directory names that write operations may not touch.
pub fn is_protected_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(AGENTS_DIR) || name.eq_ignore_ascii_case(".git")
}

/// Normalize `raw` (relative to `root`, or absolute inside it) to a `/`-separated relative
/// path. Rejects `..`, the project root itself, and `.AGENTS`/`.git`. Components that exist are
/// spelled the way the disk stores them (see [`respell`]).
pub fn normalize_rel(raw: &str, root: &Path) -> Result<String> {
    normalize(raw, root, true)
}

/// Like [`normalize_rel`] for the path a file is moved to: the folders are spelled the way the
/// disk stores them, the last name as written, because that spelling is what the caller wants the
/// file to have.
pub fn normalize_dest(raw: &str, root: &Path) -> Result<String> {
    normalize(raw, root, false)
}

fn normalize(raw: &str, root: &Path, respell_name: bool) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err(AgentError::invalid(raw, "path is empty"));
    }
    let path = Path::new(trimmed);
    let relative_buf: std::path::PathBuf;
    let relative: &Path = if path.is_absolute() {
        match path.strip_prefix(root) {
            Ok(rel) => rel,
            Err(_) => {
                if let (Ok(real_path), Ok(real_root)) =
                    (std::fs::canonicalize(path), std::fs::canonicalize(root))
                {
                    if let Ok(rel) = real_path.strip_prefix(&real_root) {
                        relative_buf = rel.to_path_buf();
                        &relative_buf
                    } else {
                        return Err(AgentError::invalid(raw, "path is outside the project root"));
                    }
                } else {
                    return Err(AgentError::invalid(raw, "path is outside the project root"));
                }
            }
        }
    } else {
        path
    };

    let mut parts: Vec<String> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => {
                return Err(AgentError::invalid(
                    raw,
                    "`..` and absolute prefixes are not allowed",
                ));
            }
        }
    }
    let Some(first) = parts.first() else {
        return Err(AgentError::invalid(
            raw,
            "path must name a file or directory inside the project",
        ));
    };
    if is_protected_name(first) {
        return Err(AgentError::ProtectedPath(parts.join("/")));
    }
    respell(root, &mut parts, respell_name);
    Ok(parts.join("/"))
}

/// Whether the filesystem that holds `root` finds a name whatever its case: the folder under its
/// own name written in the other case is the same entry. A root whose name has no letters cannot
/// say, and counts as telling names apart.
pub fn ignores_case(root: &Path) -> bool {
    let (Some(parent), Some(name)) = (
        root.parent(),
        root.file_name().and_then(|name| name.to_str()),
    ) else {
        return false;
    };
    let other = if name.chars().any(char::is_lowercase) {
        name.to_uppercase()
    } else {
        name.to_lowercase()
    };
    if other == name {
        return false;
    }
    match (
        fs::symlink_metadata(parent.join(name)),
        fs::symlink_metadata(parent.join(other)),
    ) {
        (Ok(here), Ok(there)) => same_entry(&here, &there),
        _ => false,
    }
}

/// Spell the components of `parts`, which lie under `root`, the way the filesystem stores them.
///
/// APFS and HFS+ on macOS and NTFS on Windows find a name whatever its case, so `src/Lib.rs` and
/// `src/lib.rs` are one file there while the ledger keys files by their text: two spellings would
/// give the net diff, the baseline and the known copies two entries for one file. Each existing
/// component takes the name its folder lists. A component that does not exist yet keeps the
/// spelling it was written with, and so does everything after the first missing one. On a
/// filesystem that tells names apart by case nothing changes. Symbolic links are not followed.
/// With `name_too` false the last component is left as written.
fn respell(root: &Path, parts: &mut [String], name_too: bool) {
    let mut dir = root.to_path_buf();
    let last = parts.len().saturating_sub(1);
    for (i, part) in parts.iter_mut().enumerate() {
        if fs::symlink_metadata(dir.join(&*part)).is_err() {
            return;
        }
        if (name_too || i < last)
            && let Some(stored) = stored_name(&dir, part)
        {
            *part = stored;
        }
        dir.push(&*part);
    }
}

/// The name that the folder `dir` lists for the entry `name` reaches, when it is spelled
/// differently. `None` when `name` is already the stored name or the filesystem tells names apart
/// by case.
fn stored_name(dir: &Path, name: &str) -> Option<String> {
    // Only a filesystem that ignores case finds the entry under another spelling.
    let other = if name.chars().any(char::is_lowercase) {
        name.to_uppercase()
    } else {
        name.to_lowercase()
    };
    if other == name {
        return None;
    }
    let here = fs::symlink_metadata(dir.join(name)).ok()?;
    let there = fs::symlink_metadata(dir.join(&other)).ok()?;
    if !same_entry(&here, &there) {
        return None;
    }
    let folded = name.to_lowercase();
    let mut stored = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let Ok(listed) = entry.file_name().into_string() else {
            continue;
        };
        if listed == name {
            return None;
        }
        if listed.to_lowercase() == folded {
            stored = Some(listed);
        }
    }
    stored
}

#[cfg(unix)]
fn same_entry(a: &Metadata, b: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_entry(_: &Metadata, _: &Metadata) -> bool {
    true
}

/// True when `path` equals `spec` or lies inside the directory `spec`.
pub fn matches_pathspec(path: &str, spec: &str) -> bool {
    path == spec
        || path
            .strip_prefix(spec)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Normalize path filters; `.` and empty entries mean "everything".
pub fn pathspecs(paths: Option<&[String]>, root: &Path) -> Result<Vec<String>> {
    paths
        .unwrap_or_default()
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty() && *p != ".")
        .map(|p| normalize_rel(p, root))
        .collect()
}

/// True when `specs` is empty (everything) or `path` matches one of them.
pub fn in_pathspecs(path: &str, specs: &[String]) -> bool {
    specs.is_empty() || specs.iter().any(|s| matches_pathspec(path, s))
}
