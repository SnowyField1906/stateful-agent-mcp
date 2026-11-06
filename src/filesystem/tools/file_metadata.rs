//! `filesystem_file_metadata`: metadata and text properties relevant to editing.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::common::fs::resolve::Scope;
use crate::common::fs::walk::{mode_of, mtime_of};
use crate::common::text::{human_size, perms};
use crate::common::{AgentError, Result};
use crate::workflow::Project;

pub const DESCRIPTION: &str = "Show size, permissions and modification time of a file or directory. For text files also line count, line endings, final newline and the last turn that changed it.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FileMetadataParams {
    /// File or directory, relative to the project root or absolute. Default `.`.
    #[serde(default)]
    pub implicit_path: Option<String>,
}

const MAX_SCAN_BYTES: u64 = 16 << 20;

pub fn file_metadata(project: Option<&Project>, p: FileMetadataParams) -> Result<String> {
    let scope = Scope::new(
        project.map(Project::root_dir),
        p.implicit_path.as_deref(),
        "implicit_path",
    )?;
    let target = &scope.target;
    let meta = std::fs::symlink_metadata(&target.abs)
        .map_err(|_| AgentError::PathNotFound(target.shown().to_string()))?;
    let ty = meta.file_type();
    let type_char = if ty.is_symlink() {
        'l'
    } else if ty.is_dir() {
        'd'
    } else {
        '-'
    };

    let mut out = vec![format!("path: {}", target.shown())];
    if ty.is_symlink() {
        let to = std::fs::read_link(&target.abs)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push(format!("kind: symlink -> {to}"));
    } else if ty.is_dir() {
        out.push("kind: dir".into());
    } else {
        out.push("kind: file".into());
    }
    out.push(format!("size: {} ({})", meta.len(), human_size(meta.len())));
    out.push(format!("mode: {}", perms(mode_of(&meta), type_char)));
    out.push(format!("modified: {}", mtime_of(&meta)));

    if ty.is_dir() {
        let count = std::fs::read_dir(&target.abs)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| !matches!(e.file_name().to_str(), Some(".git" | ".AGENTS")))
                    .count()
            })
            .unwrap_or(0);
        out.push(format!("entries: {count}"));
    } else if ty.is_file() {
        out.extend(text_properties(&target.abs, meta.len()));
    }
    if !target.is_root()
        && scope.in_project
        && let Some(project) = project
        && let Some(turn) = project.last_turn_touching(&target.rel)
    {
        out.push(format!("last_changed_in: {turn}"));
    }
    Ok(out.join("\n") + "\n")
}

fn text_properties(path: &std::path::Path, size: u64) -> Vec<String> {
    if size > MAX_SCAN_BYTES {
        return vec!["text: not scanned (over 16 MiB)".into()];
    }
    let Ok(bytes) = std::fs::read(path) else {
        return vec!["text: unreadable".into()];
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        let magic: String = bytes.iter().take(8).map(|b| format!("{b:02x}")).collect();
        return vec![format!("text: no (binary or non-UTF-8), magic {magic}")];
    };
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    let eol = match (lf, crlf) {
        (0, 0) => "none",
        (_, 0) => "lf",
        (0, _) => "crlf",
        _ => "mixed",
    };
    vec![
        "text: utf-8".into(),
        format!("lines: {}", text.lines().count()),
        format!("eol: {eol}"),
        format!(
            "final_newline: {}",
            if text.ends_with('\n') { "yes" } else { "no" }
        ),
    ]
}
