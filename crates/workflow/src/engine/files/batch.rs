//! `Project::apply_files`: runs a batch on the overlay, commits it and records the net result in
//! the active turn.

use serde::Serialize;

use crate::common::{AgentError, Result};
use crate::engine::Project;
use crate::engine::ops::net_ops::net_ops;
use crate::engine::ops::overlay::{Overlay, read_text_opt};
use crate::engine::ops::symbol_edit::has_syntax_errors;
use crate::engine::ops::{FileOp, Tree};
use crate::engine::session::store::{self, DeletedMap};
use crate::language::registry;
use crate::toolchain::format::{FormatReport, format_paths};

use super::normalize::normalize_op;
use super::op::BatchTracker;

/// Outcome of one batch; serializes to the fields `write_files` and `new_turn` report.
#[derive(Debug, Serialize)]
pub(crate) struct Batch {
    /// One summary line per operation.
    pub updates: Vec<String>,
    /// Only reported when the operations left every file as it was.
    #[serde(skip_serializing_if = "is_true")]
    pub changed: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub format_warnings: Vec<String>,
    /// Symbol edits whose file still has a parse error that it did not have before the edit.
    /// A warning only: an edit can be one step of a change that is valid once complete.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub syntax_warnings: Vec<String>,
    /// Exact file paths changed by this batch; used internally without duplicating MCP output.
    #[serde(skip)]
    pub changed_paths: Vec<String>,
}

impl Batch {
    /// A batch whose operations left every file as it was.
    fn unchanged(updates: Vec<String>) -> Self {
        Self {
            updates,
            changed: false,
            format_warnings: Vec::new(),
            syntax_warnings: Vec::new(),
            changed_paths: Vec::new(),
        }
    }
}

fn is_true(flag: &bool) -> bool {
    *flag
}

impl Project {
    /// Apply `ops` to the files of the active turn as one atomic batch.
    pub(crate) fn apply_files(&mut self, ops: &[FileOp]) -> Result<Batch> {
        if ops.is_empty() {
            return Err(AgentError::invalid(
                "ops",
                "at least one operation is required",
            ));
        }
        let flat = self.flat_keys();
        let mut overlay = Overlay::new(self.root.clone());
        let mut tracker = BatchTracker::default();
        let mut updates = Vec::with_capacity(ops.len());
        for (i, raw) in ops.iter().enumerate() {
            let applied = normalize_op(raw, &self.root, &overlay)
                .and_then(|op| self.apply_op(&mut overlay, &mut tracker, &flat, &op));
            updates.push(applied.map_err(|e| in_op(i, e))?);
        }

        if overlay.changed().next().is_none() {
            return Ok(Batch::unchanged(updates));
        }
        let syntax_warnings = syntax_warnings(&overlay, tracker.syntax_suspects);
        if let Err(err) = overlay.commit() {
            overlay.revert();
            return Err(err);
        }
        // Formatters rewrite files on disk; the net diff below then records the formatted text.
        let format_report = self.format_written(&overlay);
        self.track_changes(&overlay, tracker.moves, tracker.deleted);
        self.refresh_active_updates()?;
        Ok(Batch {
            updates,
            changed: true,
            format_warnings: format_report.warnings,
            syntax_warnings,
            changed_paths: overlay.changed().map(|(path, _)| path.clone()).collect(),
        })
    }

    /// Remember what a committed batch touched: the baseline of each changed file, and the moves
    /// and deletions that the net diff needs.
    fn track_changes(
        &mut self,
        overlay: &Overlay,
        moves: Vec<(String, String)>,
        deleted: Vec<String>,
    ) {
        // Only changed files get a baseline. A path that was merely probed, such as the
        // destination of a directory move, must not: it is a directory on disk after the commit.
        for (path, entry) in overlay.changed() {
            self.active_baseline
                .entry(path.clone())
                .or_insert_with(|| entry.baseline.clone());
        }
        for (from, to) in moves {
            let orig = self.active_moves.remove(&from).unwrap_or(from);
            if orig != to && self.active_baseline.get(&orig).is_some_and(Option::is_some) {
                self.active_moves.insert(to, orig);
            }
        }
        for path in deleted {
            self.active_moves.remove(&path);
        }
    }

    /// Run the configured formatters over the files the batch created or changed.
    fn format_written(&self, overlay: &Overlay) -> FormatReport {
        let paths: Vec<String> = overlay
            .changed()
            .filter(|(_, entry)| entry.current.is_some())
            .map(|(path, _)| path.clone())
            .collect();
        match self.language_config() {
            Ok(config) => format_paths(&config, &self.root, &paths),
            Err(e) => FormatReport {
                formatted: Vec::new(),
                warnings: vec![format!("formatting skipped: {e}")],
            },
        }
    }

    /// Recompute the active turn's net `updates` from `active_baseline` and the files on disk,
    /// sync `.AGENTS/.deleted/`, save the session, and remember the files in `.AGENTS/.known/`.
    fn refresh_active_updates(&mut self) -> Result<()> {
        let key = self.active;
        let mut current = Tree::new();
        for path in self.active_baseline.keys() {
            current.insert(path.clone(), read_text_opt(&self.root, path)?);
        }
        let updates = net_ops(&self.active_baseline, &current, &self.active_moves);

        let mut new_deleted = DeletedMap::new();
        for op in &updates {
            if let FileOp::Delete { path } = op
                && let Some(Some(old_text)) = self.active_baseline.get(path)
            {
                new_deleted.insert(path.clone(), old_text.clone());
            }
        }
        store::sync_turn_deleted(&self.deleted_dir, key, self.deleted.get(&key), &new_deleted)?;
        if new_deleted.is_empty() {
            self.deleted.remove(&key);
        } else {
            self.deleted.insert(key, new_deleted);
        }

        self.active_turn_mut()?.updates = updates;
        self.save_session(key.session)?;
        self.record_known(&current)
    }
}

fn in_op(index: usize, err: AgentError) -> AgentError {
    AgentError::invalid(format!("ops[{index}]"), err.to_string())
}

/// Warnings for the suspect symbol edits whose file still has a parse error at the end of the
/// batch; a later operation of the batch may have completed the change.
fn syntax_warnings(ov: &Overlay, suspects: Vec<(String, String)>) -> Vec<String> {
    suspects
        .into_iter()
        .filter(|(path, _)| {
            ov.changed().any(|(changed, entry)| {
                changed == path
                    && registry::for_path(path)
                        .zip(entry.current.as_deref())
                        .is_some_and(|(syntax, text)| has_syntax_errors(syntax, text))
            })
        })
        .map(|(_, summary)| {
            format!("{summary}: leaves a syntax error that the file did not have before")
        })
        .collect()
}
