//! One operation of a batch, applied to the overlay.

use crate::common::{AgentError, Result};
use crate::engine::Project;
use crate::engine::ops::elide::stand_ins_absent;
use crate::engine::ops::overlay::Overlay;
use crate::engine::ops::patch::{History, Version, apply_hunks_tolerant};
use crate::engine::ops::rename::rename_symbol;
use crate::engine::ops::summary::{op_summary, symbol_edit_summary};
use crate::engine::ops::symbol_edit::{Placement, edit_symbol};
use crate::engine::ops::{FileOp, PatchHunk};
use crate::engine::session::model::{TurnKey, TurnRecord};
use crate::engine::session::seen::{Origin, Sighting};

/// How many of the newest turns that changed a file are asked for the text before them.
const LEDGER_VERSIONS: usize = 3;

/// What the operations of a batch have done that the overlay alone does not tell.
#[derive(Default)]
pub(super) struct BatchTracker {
    /// File-level moves `(from_path, to_path)` performed in this batch.
    pub(super) moves: Vec<(String, String)>,
    /// Paths deleted in this batch.
    pub(super) deleted: Vec<String>,
    /// `(path, summary)` of symbol edits that added a parse error to the file when applied.
    pub(super) syntax_suspects: Vec<(String, String)>,
    /// Warnings for placeholder lines that a `Patch` wrote as text (see `stand_ins_absent`).
    pub(super) placeholder_warnings: Vec<String>,
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
            FileOp::Patch { path, hunks } => {
                let versions = || self.quotable_versions(flat, path);
                patch(
                    ov,
                    path,
                    hunks,
                    &History::new(&versions),
                    &mut tracker.placeholder_warnings,
                )
            }
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

    /// The versions of `path` that the agent was shown and may be quoting, newest first, each with
    /// what changed the file since: those a read tool showed, then the text before each of the
    /// newest turns that changed the file, which the agent may have read before a restart.
    fn quotable_versions(&self, flat: &[TurnKey], path: &str) -> Vec<Version> {
        let mut versions: Vec<Version> = self
            .seen_versions(path)
            .into_iter()
            .map(|sighting| Version {
                cause: self.cause_since(path, &sighting),
                text: sighting.text,
            })
            .collect();
        let touching = flat.iter().enumerate().rev().filter(|(_, key)| {
            self.turn(**key)
                .is_some_and(|turn| turn.updates.iter().any(|op| op.touches_path(path)))
        });
        for (at, key) in touching.take(LEDGER_VERSIONS) {
            let Ok(Some(text)) = self.content_before(flat, at, path) else {
                continue;
            };
            if versions.iter().all(|known| known.text != text) {
                versions.push(Version {
                    text,
                    cause: format!("your turn {key}"),
                });
            }
        }
        versions
    }

    /// Who changed `path` since the agent was shown `seen`.
    fn cause_since(&self, path: &str, seen: &Sighting) -> String {
        if seen.origin == Origin::PreFormat {
            return "formatter".to_string();
        }
        match self.last_key_touching(path) {
            Some(key) if Some(key.to_string()) != seen.last_turn => {
                if self.turn(key).is_some_and(TurnRecord::is_external_edit) {
                    "outside".to_string()
                } else {
                    format!("your turn {key}")
                }
            }
            _ => "outside".to_string(),
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

/// Apply `hunks` to the file; the summary lists the hunks as they matched, and how each hunk
/// that did not match byte for byte was placed.
fn patch(
    ov: &mut Overlay,
    path: &str,
    hunks: &[PatchHunk],
    history: &History<'_>,
    warnings: &mut Vec<String>,
) -> Result<String> {
    if hunks.is_empty() {
        return Err(AgentError::invalid(path, "no hunks"));
    }
    let text = ov
        .get(path)?
        .ok_or_else(|| AgentError::PathNotFound(path.to_string()))?;
    let done = apply_hunks_tolerant(path, &text, hunks, history)?;
    ov.set(path, Some(done.text))?;
    for hunk in &done.hunks {
        for line in stand_ins_absent(&text, &hunk.new_text) {
            let warning = format!(
                "{path}: wrote `{line}` as text; the file had no such line, so check that it does not stand in for code that should stay"
            );
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
    }
    let mut summary = op_summary(&FileOp::Patch {
        path: path.to_string(),
        hunks: done.hunks,
    });
    if !done.repairs.is_empty() {
        let notes: Vec<String> = done
            .repairs
            .iter()
            .map(|repair| format!("h{} {}", repair.hunk, repair.how))
            .collect();
        summary.push_str(&format!(" ({})", notes.join("; ")));
    }
    Ok(summary)
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
