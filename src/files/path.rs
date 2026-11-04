//! Project-relative path validation for write operations.

use std::path::{Component, Path};

use crate::error::{AgentError, Result};

/// Directory names that write operations may not touch.
pub fn is_protected_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(".AGENTS") || name.eq_ignore_ascii_case(".git")
}

/// Normalize `raw` (relative to `root`, or absolute inside it) to a `/`-separated relative
/// path. Rejects `..`, the project root itself, and `.AGENTS`/`.git`.
pub fn normalize_rel(raw: &str, root: &Path) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err(AgentError::invalid(raw, "path is empty"));
    }
    let path = Path::new(trimmed);
    let relative = if path.is_absolute() {
        path.strip_prefix(root)
            .map_err(|_| AgentError::invalid(raw, "path is outside the project root"))?
    } else {
        path
    };

    let mut parts: Vec<String> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return Err(AgentError::invalid(raw, "`..` and absolute prefixes are not allowed")),
        }
    }
    let Some(first) = parts.first() else {
        return Err(AgentError::invalid(raw, "path must name a file or directory inside the project"));
    };
    if is_protected_name(first) {
        return Err(AgentError::ProtectedPath(parts.join("/")));
    }
    Ok(parts.join("/"))
}

/// True when `path` equals `spec` or lies inside the directory `spec`.
pub fn matches_pathspec(path: &str, spec: &str) -> bool {
    path == spec || path.strip_prefix(spec).is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_rejects() {
        let root = Path::new("/p");
        assert_eq!(normalize_rel("./src//a.rs", root).unwrap(), "src/a.rs");
        assert_eq!(normalize_rel("/p/src/a.rs", root).unwrap(), "src/a.rs");
        assert_eq!(normalize_rel("a..b", root).unwrap(), "a..b");
        assert!(normalize_rel("../x", root).is_err());
        assert!(normalize_rel("/other/x", root).is_err());
        assert!(normalize_rel(".agents/x", root).is_err());
        assert!(normalize_rel(".git/config", root).is_err());
        assert!(normalize_rel(".", root).is_err());
    }

    #[test]
    fn pathspec() {
        assert!(matches_pathspec("src/a.rs", "src"));
        assert!(matches_pathspec("src", "src"));
        assert!(!matches_pathspec("srcs/a.rs", "src"));
    }
}
