//! `Project::prepare_files` runs a batch on the overlay; `Project::commit_files` writes it to disk
//! and records the net result in the active turn. Between the two, the caller settles the changes
//! made outside the ledger to the files the batch changes (see `external`).

use std::collections::BTreeSet;

use serde::Serialize;

use crate::common::{AgentError, Result};
use crate::engine::Project;
use crate::engine::ops::diff::line_regions;
use crate::engine::ops::net_ops::net_ops;
use crate::engine::ops::overlay::{Overlay, read_text_opt};
use crate::engine::ops::symbol_edit::has_syntax_errors;
use crate::engine::ops::{FileOp, Tree};
use crate::engine::session::seen::{Origin, Sighting};
use crate::engine::session::store::{self, DeletedMap};
use crate::language::registry;
use crate::toolchain::format::{FormatReport, format_paths};

use super::normalize::normalize_op;
use super::op::BatchTracker;

/// Most rows of `Batch::reshaped` that list rewritten lines.
const RESHAPED_ROWS: usize = 6;
/// Longest text of one line in a `reshaped` row, in characters.
const RESHAPED_TEXT: usize = 60;

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
    /// What a formatter did to the lines this batch edited: up to `RESHAPED_ROWS` rows such as
    /// `a.rs 41: fn a(x:i32) -> fn a(x: i32)`, then one entry that counts the rows left out and
    /// the lines it rewrote outside the edited ranges.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reshaped: Vec<String>,
    /// Symbol edits whose file still has a parse error that it did not have before the edit.
    /// A warning only: an edit can be one step of a change that is valid once complete.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub syntax_warnings: Vec<String>,
    /// Placeholder lines such as `// ... existing code ...` that a `Patch` wrote as text because
    /// the file did not hold them: they may stand in for code the agent meant to keep.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub placeholder_warnings: Vec<String>,
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
            reshaped: Vec::new(),
            syntax_warnings: Vec::new(),
            placeholder_warnings: Vec::new(),
            changed_paths: Vec::new(),
        }
    }
}

fn is_true(flag: &bool) -> bool {
    *flag
}

/// A batch that ran on the overlay and is not on disk yet.
pub(crate) struct Prepared {
    overlay: Overlay,
    tracker: BatchTracker,
    updates: Vec<String>,
}

impl Prepared {
    /// The files the batch changes.
    pub(crate) fn changed(&self) -> BTreeSet<String> {
        self.overlay
            .changed()
            .map(|(path, _)| path.clone())
            .collect()
    }
}

impl Project {
    /// Run `ops` on an overlay of the files as one atomic batch. Nothing is written or recorded:
    /// a failing operation fails the batch here.
    pub(crate) fn prepare_files(&self, ops: &[FileOp]) -> Result<Prepared> {
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
        Ok(Prepared {
            overlay,
            tracker,
            updates,
        })
    }

    /// Write a prepared batch to the files of the active turn and record its net result.
    pub(crate) fn commit_files(&mut self, prepared: Prepared) -> Result<Batch> {
        let Prepared {
            overlay,
            tracker,
            updates,
        } = prepared;
        if overlay.changed().next().is_none() {
            return Ok(Batch::unchanged(updates));
        }
        let syntax_warnings = syntax_warnings(&overlay, tracker.syntax_suspects);
        let placeholder_warnings = tracker.placeholder_warnings;
        if let Err(err) = overlay.commit() {
            overlay.revert();
            return Err(err);
        }
        // Formatters rewrite files on disk; the net diff below then records the formatted text.
        let format_report = self.format_written(&overlay);
        let mut reshaped = Vec::new();
        if !format_report.formatted.is_empty() {
            self.note_pre_format(&overlay);
            reshaped = self.reshaped(&overlay);
        }
        self.track_changes(&overlay, tracker.moves, tracker.deleted);
        self.refresh_active_updates()?;
        Ok(Batch {
            updates,
            changed: true,
            format_warnings: format_report.warnings,
            reshaped,
            syntax_warnings,
            placeholder_warnings,
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

    /// Keep the text of each file as the agent wrote it when the formatter has changed it, so that
    /// a later patch quoting that text can still be placed.
    fn note_pre_format(&self, overlay: &Overlay) {
        for (path, entry) in overlay.changed() {
            let Some(written) = &entry.current else {
                continue;
            };
            if matches!(read_text_opt(&self.root, path), Ok(Some(after)) if after != *written) {
                self.remember_seen(
                    path,
                    Sighting {
                        text: written.clone(),
                        origin: Origin::PreFormat,
                        last_turn: Some(self.active.to_string()),
                    },
                );
            }
        }
    }

    /// Echo what the formatter did to the lines this batch edited, so that the next quote of them is
    /// right: a row per rewritten line inside the edited ranges, and a count of the lines it rewrote
    /// elsewhere in the same files.
    fn reshaped(&self, overlay: &Overlay) -> Vec<String> {
        let mut rows = Vec::new();
        let mut outside = 0;
        for (path, entry) in overlay.changed() {
            let Some(written) = &entry.current else {
                continue;
            };
            let Ok(Some(after)) = read_text_opt(&self.root, path) else {
                continue;
            };
            let edited = edited_lines(entry.baseline.as_deref(), written);
            let (file_rows, file_outside) = reshaped_in(path, written, &after, &edited);
            rows.extend(file_rows);
            outside += file_outside;
        }
        with_tally(rows, outside)
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

/// `err` of operation `index`. A patch conflict keeps its kind: the server reads it to hint after
/// repeated misses on one file.
fn in_op(index: usize, err: AgentError) -> AgentError {
    match err {
        AgentError::PatchConflict { path, reason } => AgentError::PatchConflict {
            path,
            reason: format!("ops[{index}] {reason}"),
        },
        other => AgentError::invalid(format!("ops[{index}]"), other.to_string()),
    }
}

/// The 0-based, end-exclusive line ranges of `written` that differ from `baseline`.
fn edited_lines(baseline: Option<&str>, written: &str) -> Vec<(usize, usize)> {
    line_regions(baseline.unwrap_or(""), written)
        .into_iter()
        .map(|(_, _, first, end)| (first, end))
        .filter(|(first, end)| first < end)
        .collect()
}

/// Whether lines `from..to` (0-based, end exclusive) of the written text touch an edited range.
fn is_edited(edited: &[(usize, usize)], from: usize, to: usize) -> bool {
    edited.iter().any(|&(first, end)| from < end && first < to)
}

/// The rows for the lines that a formatter rewrote in one file, and how many rewritten lines lie
/// outside the edited ranges. A region that kept its line count is judged and shown line by line;
/// one that joined or split lines is one row, or one outside count.
fn reshaped_in(
    path: &str,
    written: &str,
    after: &str,
    edited: &[(usize, usize)],
) -> (Vec<String>, usize) {
    let before: Vec<&str> = written.split_inclusive('\n').collect();
    let now: Vec<&str> = after.split_inclusive('\n').collect();
    let mut rows = Vec::new();
    let mut outside = 0;
    for (o0, o1, n0, n1) in line_regions(written, after) {
        if o1 - o0 != n1 - n0 {
            if is_edited(edited, o0, o1.max(o0 + 1)) {
                let (first, last) = (n0 + 1, n1.max(n0 + 1));
                let at = if first == last {
                    first.to_string()
                } else {
                    format!("{first}..{last}")
                };
                rows.push(format!(
                    "{path} {at}: {} lines -> {} lines",
                    o1 - o0,
                    n1 - n0
                ));
            } else {
                outside += (o1 - o0).max(n1 - n0);
            }
            continue;
        }
        for k in 0..o1 - o0 {
            let (old, new) = (before[o0 + k].trim_end(), now[n0 + k].trim_end());
            if old == new {
                continue;
            }
            if is_edited(edited, o0 + k, o0 + k + 1) {
                rows.push(format!("{path} {}: {}", n0 + k + 1, change(old, new)));
            } else {
                outside += 1;
            }
        }
    }
    (rows, outside)
}

/// One rewritten line as `before -> after`, or `whitespace` when only the spacing changed.
fn change(old: &str, new: &str) -> String {
    if old.trim() == new.trim() {
        return "whitespace".to_string();
    }
    format!("{} -> {}", clip(old.trim()), clip(new.trim()))
}

fn clip(text: &str) -> String {
    match text.char_indices().nth(RESHAPED_TEXT) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

/// The first `RESHAPED_ROWS` of `rows`, then one entry that counts the rows left out and the
/// `outside` lines rewritten outside the edited ranges.
fn with_tally(mut rows: Vec<String>, outside: usize) -> Vec<String> {
    let left_out = rows.len().saturating_sub(RESHAPED_ROWS);
    rows.truncate(RESHAPED_ROWS);
    let mut tally = Vec::new();
    if left_out > 0 {
        tally.push(format!("{left_out} more rows"));
    }
    if outside > 0 {
        tally.push(format!(
            "{outside} lines rewritten outside the edited ranges"
        ));
    }
    if !tally.is_empty() {
        rows.push(tally.join(", "));
    }
    rows
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
