//! `langtool_read_file`: numbered line ranges of multiple text files (with optional AST block
//! folding via `fold_level`), now or as of a past turn.

use crate::error::{AgentError, Result};
use crate::lang::{depth, language_for_path};
use crate::params::{FsReadFilesParams, Range};
use crate::state::Project;

use super::fmt::clip;
use super::resolve::{Scope, Target};
use super::scan::UNSUPPORTED;

const DEFAULT_LINES: u64 = 400;
const MAX_LINES: u64 = 2000;
const MAX_FILE_BYTES: u64 = 16 << 20;
const LINE_CLIP: usize = 500;
const MAX_FOLD_LEVEL: u64 = 16;

pub fn read_files(project: Option<&Project>, p: FsReadFilesParams) -> Result<String> {
    if p.files.is_empty() || p.files.len() > 32 {
        return Err(AgentError::invalid("files", "provide 1 to 32 file requests"));
    }
    let mut out = String::new();
    for file in p.files {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&read_one(
            project,
            &file.file_path,
            file.lines,
            file.numbered,
            file.fold_level,
            p.at_implicit_id.as_deref(),
        )?);
    }
    Ok(out)
}

fn read_one(
    project: Option<&Project>,
    file_path: &str,
    selected_range: Option<Range>,
    numbered: Option<bool>,
    fold_level: Option<u64>,
    at_implicit_id: Option<&str>,
) -> Result<String> {
    if file_path.trim().is_empty() {
        return Err(AgentError::invalid("file_path", "must not be empty"));
    }
    let scope = Scope::new(project, Some(file_path))?;
    let target = &scope.target;
    if target.is_root() {
        return Err(AgentError::invalid("file_path", "must be a file"));
    }
    let at = at_implicit_id.map(str::trim).filter(|s| !s.is_empty());
    let text = match at {
        Some(id) => {
            let project = scope.project.ok_or_else(|| {
                AgentError::invalid(
                    "at_implicit_id",
                    "applies to project paths; it cannot be combined with paths outside the registered project",
                )
            })?;
            project.read_at(id, &target.rel)?.ok_or_else(|| {
                AgentError::NotFound(format!("{} did not exist at {id}", target.rel))
            })?
        }
        None => read_text(target)?,
    };

    if let Some(raw_level) = fold_level {
        let syntax = language_for_path(target.shown())
            .and_then(|l| l.syntax)
            .ok_or_else(|| AgentError::invalid("file_path", UNSUPPORTED))?;
        let total = text.lines().count();
        if total == 0 {
            return Ok(format!("{} (empty)\n", target.shown()));
        }
        let max_depth = raw_level.min(MAX_FOLD_LEVEL) as usize;
        let rows = depth::fold(syntax, &text, max_depth)
            .ok_or_else(|| AgentError::invalid("file_path", "could not be parsed"))?;
        let (start, count) = match selected_range {
            None => (0, DEFAULT_LINES as usize),
            Some(range) => match range.tail() {
                Some(n) => {
                    let n = n.clamp(1, MAX_LINES) as usize;
                    (total.saturating_sub(n), n)
                }
                None => range.forward(DEFAULT_LINES, MAX_LINES, "lines")?,
            },
        };
        if start >= total {
            return Err(AgentError::invalid(
                "lines",
                format!("{} has {total} lines", target.shown()),
            ));
        }
        let window_end = (start + count).min(total);
        let shown: Vec<depth::Row> = rows
            .into_iter()
            .filter(|r| r.last > start && r.first <= window_end)
            .collect();
        let first = shown.first().map_or(start + 1, |r| r.first);
        let end = shown.last().map_or(window_end, |r| r.last).max(window_end);

        let mut out = format!(
            "{} [{first}..{end} of {total}] fold_level {max_depth}",
            target.shown()
        );
        if text.contains("\r\n") {
            out.push_str(" crlf");
        }
        if let Some(id) = at {
            out.push_str(&format!(" @{id}"));
        }
        if end < total {
            out.push_str(&format!(" next lines={}", Range::next(end, count)));
        }
        out.push('\n');
        if numbered.unwrap_or(true) {
            out.push_str(&depth::render(&shown));
        } else {
            for row in &shown {
                out.push_str(&row.text);
                out.push('\n');
            }
        }
        return Ok(out);
    }

    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total == 0 {
        return Ok(format!("{} (empty)\n", target.shown()));
    }
    let (start, count) = match selected_range {
        None => (0, DEFAULT_LINES as usize),
        Some(range) => match range.tail() {
            Some(n) => {
                let n = n.clamp(1, MAX_LINES) as usize;
                (total.saturating_sub(n), n)
            }
            None => range.forward(DEFAULT_LINES, MAX_LINES, "lines")?,
        },
    };
    if start >= total {
        return Err(AgentError::invalid(
            "lines",
            format!("{} has {total} lines", target.shown()),
        ));
    }
    let end = (start + count).min(total);

    let mut out = format!("{} [{}..{end} of {total}]", target.shown(), start + 1);
    if text.contains("\r\n") {
        out.push_str(" crlf");
    }
    if let Some(id) = at {
        out.push_str(&format!(" @{id}"));
    }
    if end < total {
        out.push_str(&format!(" next lines={}", Range::next(end, count)));
    }
    out.push('\n');
    let numbered = numbered.unwrap_or(true);
    for (i, line) in lines[start..end].iter().enumerate() {
        if numbered {
            out.push_str(&format!("{}\t", start + i + 1));
        }
        out.push_str(&clip(line, LINE_CLIP));
        out.push('\n');
    }
    Ok(out)
}

pub(super) fn read_text(target: &Target) -> Result<String> {
    let meta = std::fs::metadata(&target.abs).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AgentError::PathNotFound(target.shown().to_string()),
        _ => AgentError::io_at(&target.abs, e),
    })?;
    if meta.is_dir() {
        return Err(AgentError::invalid(
            &target.rel,
            "is a directory; use filesystem_ls",
        ));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(AgentError::invalid(
            &target.rel,
            format!(
                "{} bytes; files over {MAX_FILE_BYTES} bytes are not read",
                meta.len()
            ),
        ));
    }
    let bytes = std::fs::read(&target.abs).map_err(|e| AgentError::io_at(&target.abs, e))?;
    String::from_utf8(bytes)
        .map_err(|_| AgentError::invalid(&target.rel, "not valid UTF-8 text (binary file?)"))
}
