//! Path resolution for the filesystem tools.

use std::path::{Component, Path, PathBuf};

use crate::error::{AgentError, Result};
use crate::files::path::normalize_rel;
use crate::state::Project;

/// A user-supplied path resolved against a scope root.
pub struct Target {
    pub abs: PathBuf,
    /// Relative to the scope root, `/`-separated; empty for the root itself.
    pub rel: String,
    /// Path for messages and headers.
    label: String,
}

impl Target {
    pub fn is_root(&self) -> bool {
        self.rel.is_empty()
    }

    /// Path for messages and headers.
    pub fn shown(&self) -> &str {
        &self.label
    }
}

/// What a filesystem tool looks at: the registered project, or an absolute path on disk.
pub struct Scope<'a> {
    /// Listed names are relative to this directory.
    pub root: PathBuf,
    pub target: Target,
    /// The registered project; `None` for an absolute path outside a registered project.
    pub project: Option<&'a Project>,
}

impl<'a> Scope<'a> {
    pub fn new(project: Option<&'a Project>, raw: Option<&str>) -> Result<Self> {
        let given = raw.map(str::trim).filter(|p| !p.is_empty());
        if let Some(s) = given
            && Path::new(s).is_absolute()
        {
            if let Some(project) = project {
                if Path::new(s).starts_with(&project.root) {
                    let target = resolve(&project.root, Some(s))?;
                    return Ok(Self {
                        root: project.root.clone(),
                        target,
                        project: Some(project),
                    });
                }
                if let Ok(real) = std::fs::canonicalize(s)
                    && real.starts_with(&project.root)
                    && let Some(real_str) = real.to_str()
                {
                    let target = resolve(&project.root, Some(real_str))?;
                    return Ok(Self {
                        root: project.root.clone(),
                        target,
                        project: Some(project),
                    });
                }
            }
            let (root, target) = resolve_absolute(s)?;
            return Ok(Self {
                root,
                target,
                project: None,
            });
        }
        let project = project.ok_or_else(|| {
            AgentError::invalid(
                "path",
                "no project registered; call `workflow_register` first, or pass an absolute path",
            )
        })?;
        let target = resolve(&project.root, given)?;
        Ok(Self {
            root: project.root.clone(),
            target,
            project: Some(project),
        })
    }
}

/// Resolve `raw` (relative to `root`, or absolute inside it). Empty, `.` and the root itself
/// mean the project root. `..`, `.git` and `.AGENTS` are rejected, as are symlinks that lead
/// outside the root.
pub fn resolve(root: &Path, raw: Option<&str>) -> Result<Target> {
    let raw = raw.map(str::trim).unwrap_or("");
    let path = Path::new(raw);
    if path == root || path.components().all(|c| matches!(c, Component::CurDir)) {
        return Ok(Target { abs: root.to_path_buf(), rel: String::new(), label: ".".into() });
    }
    let rel = normalize_rel(raw, root)?;
    let abs = root.join(&rel);
    if let Ok(real) = std::fs::canonicalize(&abs)
        && !real.starts_with(root)
    {
        return Err(AgentError::invalid(raw, "resolves outside the project root"));
    }
    let label = rel.clone();
    Ok(Target { abs, rel, label })
}

/// Resolve an absolute file or directory anywhere on disk. A directory becomes the scope root;
/// for a file the root is its parent directory. Nothing is checked beyond existence, because
/// these reads are explicit and read-only.
fn resolve_absolute(raw: &str) -> Result<(PathBuf, Target)> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(AgentError::invalid("path", "must be an absolute path"));
    }
    let is_dir = std::fs::metadata(path)
        .map(|meta| meta.is_dir())
        .map_err(|_| AgentError::PathNotFound(raw.to_string()))?;
    let label = path.to_string_lossy().into_owned();
    if is_dir {
        let root = path.to_path_buf();
        let target = Target { abs: root.clone(), rel: String::new(), label };
        return Ok((root, target));
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(AgentError::invalid("path", "must name a file or directory"));
    };
    let target = Target {
        abs: path.to_path_buf(),
        rel: name.to_string_lossy().into_owned(),
        label,
    };
    Ok((parent.to_path_buf(), target))
}
