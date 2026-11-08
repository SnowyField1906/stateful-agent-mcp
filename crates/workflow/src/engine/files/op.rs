//! One operation of a batch, applied to the overlay.

use crate::common::{AgentError, Result};
use crate::engine::Project;
use crate::engine::ops::overlay::Overlay;
use crate::engine::ops::patch::apply_hunks;
use crate::engine::ops::rename::rename_symbol;
use crate::engine::ops::summary::{op_summary, symbol_edit_summary};
use crate::engine::ops::symbol_edit::{Placement, edit_symbol};
use crate::engine::ops::{FileOp, PatchHunk};
use crate::engine::session::model::TurnKey;

/// What the operations of a batch have done that the overlay alone does not tell.
#[derive(Default)]
pub(super) struct BatchTracker {
    /// File-level moves `(from_path, to_path)` performed in this batch.
    pub(super) moves: Vec<(String, String)>,
    /// Paths deleted in this batch.
    pub(super) deleted: Vec<String>,
    /// `(path, summary)` of symbol edits that added a parse error to the file when applied.
    pub(super) syntax_suspects: Vec<(String, String)>,
}

impl Project {
    /// Apply the normalized `op` to the overlay and return its summary line.
    pub(super) fn apply_op(
        &self,
        ov: &mut Overlay,
        tracker: &mut BatchTracker,
        flat: &[TurnKey],
        op: &FileOp,
    ) -> Result<String> {
        match op {
            FileOp::Create { path, content } => {
                create(ov, path, content)?;
                Ok(op_summary(op))
            }
            FileOp::Patch { path, hunks } => patch(ov, path, hunks),
            FileOp::ReplaceSymbol {
                path,
                symbol,
                content,
            } => apply_symbol_edit(ov, tracker, op, (path, symbol, content), Placement::Replace),
            FileOp::InsertBeforeSymbol {
                path,
                symbol,
                content,
            } => apply_symbol_edit(ov, tracker, op, (path, symbol, content), Placement::Before),
            FileOp::InsertAfterSymbol {
                path,
                symbol,
                content,
            } => apply_symbol_edit(ov, tracker, op, (path, symbol, content), Placement::After),
            FileOp::RenameSymbol {
                symbol,
                new_name,
                path,
            } => rename_symbol(ov, &self.root, symbol, new_name, path.as_deref()),
            FileOp::Move { path, to_path } => {
                move_path(ov, path, to_path, &mut tracker.moves)?;
                Ok(op_summary(op))
            }
            FileOp::Delete { path } => delete(ov, tracker, op, path),
            FileOp::Recover { id, to_path } => self.recover(ov, flat, id, to_path.as_deref()),
        }
    }
}

/// True for a directory on disk or one implied by files created in this batch.
pub(super) fn is_dir_view(ov: &Overlay, rel: &str) -> Result<bool> {
    Ok(ov.is_dir(rel) || !ov.list_files(rel)?.is_empty())
}

fn create(ov: &mut Overlay, path: &str, content: &str) -> Result<()> {
    if is_dir_view(ov, path)? || ov.get(path)?.is_some() {
        return Err(AgentError::invalid(
            path,
            "already exists; use Patch to edit an existing file",
        ));
    }
    ov.set(path, Some(content.to_string()))
}

/// Apply `hunks` to the file; the summary lists the hunks as they matched.
fn patch(ov: &mut Overlay, path: &str, hunks: &[PatchHunk]) -> Result<String> {
    if hunks.is_empty() {
        return Err(AgentError::invalid(path, "no hunks"));
    }
    let text = ov
        .get(path)?
        .ok_or_else(|| AgentError::PathNotFound(path.to_string()))?;
    let (patched, applied) = apply_hunks(path, &text, hunks)?;
    ov.set(path, Some(patched))?;
    Ok(op_summary(&FileOp::Patch {
        path: path.to_string(),
        hunks: applied,
    }))
}

/// Delete a file, or every file below a directory.
fn delete(ov: &mut Overlay, tracker: &mut BatchTracker, op: &FileOp, path: &str) -> Result<String> {
    if is_dir_view(ov, path)? {
        let files = ov.list_files(path)?;
        for file in &files {
            ov.set(file, None)?;
            tracker.deleted.push(file.clone());
        }
        return Ok(format!("Delete {path} ({} files)", files.len()));
    }
    if ov.get(path)?.is_none() {
        return Err(AgentError::PathNotFound(path.to_string()));
    }
    ov.set(path, None)?;
    tracker.deleted.push(path.to_string());
    Ok(op_summary(op))
}

/// Apply a symbol operation (`path`, `symbol`, `content`) to the file in the overlay.
fn apply_symbol_edit(
    ov: &mut Overlay,
    tracker: &mut BatchTracker,
    op: &FileOp,
    (path, symbol, content): (&str, &str, &str),
    placement: Placement,
) -> Result<String> {
    let text = ov
        .get(path)?
        .ok_or_else(|| AgentError::PathNotFound(path.to_string()))?;
    let edit = edit_symbol(path, &text, symbol, content, placement)
        .map_err(|reason| AgentError::invalid(path, reason))?;
    let summary = symbol_edit_summary(op, &edit.hunk);
    if edit.adds_syntax_error {
        tracker
            .syntax_suspects
            .push((path.to_string(), summary.clone()));
    }
    ov.set(path, Some(edit.text))?;
    Ok(summary)
}

fn move_path(
    ov: &mut Overlay,
    from: &str,
    to: &str,
    moves: &mut Vec<(String, String)>,
) -> Result<()> {
    if from == to {
        return Err(AgentError::invalid(
            to,
            "source and destination are the same",
        ));
    }
    if to.starts_with(&format!("{from}/")) {
        return Err(AgentError::invalid(to, "destination is inside the source"));
    }
    if is_dir_view(ov, to)? || ov.get(to)?.is_some() {
        return Err(AgentError::invalid(to, "destination already exists"));
    }
    if is_dir_view(ov, from)? {
        let files = ov.list_files(from)?;
        if files.is_empty() {
            return Err(AgentError::invalid(from, "directory has no files to move"));
        }
        for file in &files {
            let dest = format!("{to}{}", &file[from.len()..]);
            let content = ov.get(file)?;
            ov.set(&dest, content)?;
            ov.set(file, None)?;
            moves.push((file.clone(), dest));
        }
    } else {
        let content = ov
            .get(from)?
            .ok_or_else(|| AgentError::PathNotFound(from.to_string()))?;
        ov.set(to, Some(content))?;
        ov.set(from, None)?;
        moves.push((from.to_string(), to.to_string()));
    }
    Ok(())
}
