//! The batch core of `workflow_write_files`: applies file operations atomically and records the
//! turn's net changes.
//!
//! A batch is applied to an in-memory [`Overlay`]; the disk is touched only when every
//! operation succeeded. For the active turn the project keeps `active_baseline` (the content of
//! each touched file before the turn first touched it) and `active_moves` (`to_path -> from_path`).
//! After every batch the turn's `updates` are recomputed as the net difference between that
//! baseline and the files on disk, and `.AGENTS/.deleted/` is synced with any net-deleted files.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::common::pathspec::{matches_pathspec, normalize_rel};
use crate::common::{AgentError, Result};
use crate::langtool::engine::syntax::registry;
use crate::toolchain::format::{FormatReport, format_paths};

use super::Project;
use super::ops::net_ops::net_ops;
use super::ops::overlay::{Overlay, read_text_opt};
use super::ops::patch::apply_hunks;
use super::ops::rename::rename_symbol;
use super::ops::summary::{op_summary, symbol_edit_summary};
use super::ops::symbol_edit::{Placement, edit_symbol, has_syntax_errors};
use super::ops::{FileOp, Tree};
use super::selector::Sel;
use super::session::model::TurnKey;
use super::session::store::{self, DeletedMap};

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

fn is_true(flag: &bool) -> bool {
    *flag
}

#[derive(Default)]
struct BatchTracker {
    /// File-level moves `(from_path, to_path)` performed in this batch.
    moves: Vec<(String, String)>,
    /// Paths deleted in this batch.
    deleted: Vec<String>,
    /// `(path, summary)` of symbol edits that added a parse error to the file when applied.
    syntax_suspects: Vec<(String, String)>,
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
            let applied = normalize_op(raw, &self.root)
                .and_then(|op| self.apply_op(&mut overlay, &mut tracker, &flat, &op));
            updates.push(applied.map_err(|e| in_op(i, e))?);
        }

        if overlay.changed().next().is_none() {
            return Ok(Batch {
                updates,
                changed: false,
                format_warnings: Vec::new(),
                syntax_warnings: Vec::new(),
                changed_paths: Vec::new(),
            });
        }
        let syntax_warnings = syntax_warnings(&overlay, tracker.syntax_suspects);
        if let Err(err) = overlay.commit() {
            overlay.revert();
            return Err(err);
        }
        // Formatters rewrite files on disk; the net diff below then records the formatted text.
        let format_report = self.format_written(&overlay);

        // Only changed files get a baseline. A path that was merely probed, such as the
        // destination of a directory move, must not: it is a directory on disk after the commit.
        for (path, entry) in overlay.changed() {
            self.active_baseline
                .entry(path.clone())
                .or_insert_with(|| entry.baseline.clone());
        }
        for (from, to) in tracker.moves {
            let orig = self.active_moves.remove(&from).unwrap_or(from);
            if orig != to && self.active_baseline.get(&orig).is_some_and(Option::is_some) {
                self.active_moves.insert(to, orig);
            }
        }
        for del in tracker.deleted {
            self.active_moves.remove(&del);
        }

        self.refresh_active_updates()?;
        let changed_paths = overlay.changed().map(|(path, _)| path.clone()).collect();
        Ok(Batch {
            updates,
            changed: true,
            format_warnings: format_report.warnings,
            syntax_warnings,
            changed_paths,
        })
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
    /// sync `.AGENTS/.deleted/`, and save the session.
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
        self.save_session(key.session)
    }

    fn apply_op(
        &self,
        ov: &mut Overlay,
        tracker: &mut BatchTracker,
        flat: &[TurnKey],
        op: &FileOp,
    ) -> Result<String> {
        match op {
            FileOp::Create { path, content } => {
                if is_dir_view(ov, path)? || ov.get(path)?.is_some() {
                    return Err(AgentError::invalid(
                        path,
                        "already exists; use Patch to edit an existing file",
                    ));
                }
                ov.set(path, Some(content.clone()))?;
                Ok(op_summary(op))
            }
            FileOp::Patch { path, hunks } => {
                if hunks.is_empty() {
                    return Err(AgentError::invalid(path, "no hunks"));
                }
                let text = ov
                    .get(path)?
                    .ok_or_else(|| AgentError::PathNotFound(path.clone()))?;
                let (patched, applied) = apply_hunks(path, &text, hunks)?;
                ov.set(path, Some(patched))?;
                Ok(op_summary(&FileOp::Patch {
                    path: path.clone(),
                    hunks: applied,
                }))
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
            FileOp::Delete { path } => {
                if is_dir_view(ov, path)? {
                    let files = ov.list_files(path)?;
                    for file in &files {
                        ov.set(file, None)?;
                        tracker.deleted.push(file.clone());
                    }
                    return Ok(format!("Delete {path} ({} files)", files.len()));
                }
                if ov.get(path)?.is_none() {
                    return Err(AgentError::PathNotFound(path.clone()));
                }
                ov.set(path, None)?;
                tracker.deleted.push(path.clone());
                Ok(op_summary(op))
            }
            FileOp::Recover { id, to_path } => self.recover(ov, flat, id, to_path.as_deref()),
        }
    }

    /// Restore files to their content at a past turn.
    /// - update ID (`01.02.update.src/a.rs`): restores that file (optionally into `to_path`)
    /// - turn, session or range with `to_path`: restores that file or directory prefix
    /// - turn, session or range without `to_path`: restores every file those turns touched
    fn recover(
        &self,
        ov: &mut Overlay,
        flat: &[TurnKey],
        id: &str,
        to_path: Option<&str>,
    ) -> Result<String> {
        match self.parse_selector(flat, id)? {
            Sel::Update(key, path) => {
                let idx = flat
                    .iter()
                    .position(|k| *k == key)
                    .ok_or_else(|| AgentError::NotFound(format!("turn {key}")))?;
                let path = normalize_rel(&path, &self.root)?;
                let turn_touched = self
                    .turn(key)
                    .is_some_and(|t| t.updates.iter().any(|op| op.touches_path(&path)))
                    || self
                        .deleted
                        .get(&key)
                        .is_some_and(|m| m.contains_key(&path));
                if !turn_touched {
                    return Err(AgentError::NotFound(id.trim().to_string()));
                }
                let target = to_path.unwrap_or(&path);
                if is_dir_view(ov, target)? {
                    return Err(AgentError::invalid(target, "is a directory"));
                }
                let mut content = self.content_before(flat, idx + 1, &path)?;
                if content.is_none() && ov.get(target)?.is_none() {
                    content = self.content_before(flat, idx, &path)?;
                }
                let existed = content.is_some();
                ov.set(target, content)?;
                Ok(format!(
                    "Recover {id} → {target}{}",
                    if existed { "" } else { " (deleted)" }
                ))
            }
            Sel::Turns { lo, hi, reversed } => {
                let target_idx = if reversed { lo + 1 } else { hi + 1 };
                if let Some(spec) = to_path {
                    return self.recover_pathspec(ov, flat, id, lo, target_idx, spec);
                }
                let mut paths = BTreeSet::new();
                for key in &flat[lo..=hi] {
                    paths.extend(self.touched_paths(*key));
                }
                for path in &paths {
                    let mut content = self.content_before(flat, target_idx, path)?;
                    if content.is_none() && ov.get(path)?.is_none() {
                        content = self.content_before(flat, lo, path)?;
                    }
                    ov.set(path, content)?;
                }
                Ok(format!("Recover {id} ({} files)", paths.len()))
            }
            _ => Err(AgentError::invalid(
                "id",
                "expected an update, turn, session or range ID",
            )),
        }
    }

    fn recover_pathspec(
        &self,
        ov: &mut Overlay,
        flat: &[TurnKey],
        id: &str,
        lo: usize,
        target_idx: usize,
        spec: &str,
    ) -> Result<String> {
        let mut candidates = BTreeSet::new();
        for key in flat {
            for p in self.touched_paths(*key) {
                if matches_pathspec(&p, spec) {
                    candidates.insert(p);
                }
            }
        }
        let spec_is_dir = is_dir_view(ov, spec)?;
        if spec_is_dir {
            for p in ov.list_files(spec)? {
                candidates.insert(p);
            }
        }
        let is_prefix_match = spec_is_dir
            || candidates
                .iter()
                .any(|p| p != spec && matches_pathspec(p, spec));
        if is_prefix_match {
            for path in &candidates {
                let mut content = self.content_before(flat, target_idx, path)?;
                if content.is_none() && ov.get(path)?.is_none() {
                    content = self.content_before(flat, lo, path)?;
                }
                ov.set(path, content)?;
            }
            return Ok(format!(
                "Recover {id} → {spec} ({} files)",
                candidates.len()
            ));
        }

        let mut content = self.content_before(flat, target_idx, spec)?;
        if content.is_none() && ov.get(spec)?.is_none() {
            content = self.content_before(flat, lo, spec)?;
        }
        if content.is_none() && ov.get(spec)?.is_none() && !candidates.contains(spec) {
            return Err(AgentError::NotFound(format!("{spec} at {id}")));
        }
        let existed = content.is_some();
        ov.set(spec, content)?;
        Ok(format!(
            "Recover {id} → {spec}{}",
            if existed { "" } else { " (deleted)" }
        ))
    }
}

fn in_op(index: usize, err: AgentError) -> AgentError {
    AgentError::invalid(format!("ops[{index}]"), err.to_string())
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

/// Validate and normalize every path of `op`.
fn normalize_op(op: &FileOp, root: &Path) -> Result<FileOp> {
    let norm = |p: &str| normalize_rel(p, root);
    Ok(match op {
        FileOp::Create { path, content } => FileOp::Create {
            path: norm(path)?,
            content: content.clone(),
        },
        FileOp::Patch { path, hunks } => FileOp::Patch {
            path: norm(path)?,
            hunks: hunks.clone(),
        },
        FileOp::Delete { path } => FileOp::Delete { path: norm(path)? },
        FileOp::ReplaceSymbol {
            path,
            symbol,
            content,
        } => FileOp::ReplaceSymbol {
            path: norm(path)?,
            symbol: symbol.trim().to_string(),
            content: content.clone(),
        },
        FileOp::InsertBeforeSymbol {
            path,
            symbol,
            content,
        } => FileOp::InsertBeforeSymbol {
            path: norm(path)?,
            symbol: symbol.trim().to_string(),
            content: content.clone(),
        },
        FileOp::InsertAfterSymbol {
            path,
            symbol,
            content,
        } => FileOp::InsertAfterSymbol {
            path: norm(path)?,
            symbol: symbol.trim().to_string(),
            content: content.clone(),
        },
        FileOp::RenameSymbol {
            symbol,
            new_name,
            path,
        } => FileOp::RenameSymbol {
            symbol: symbol.trim().to_string(),
            new_name: new_name.trim().to_string(),
            path: match path.as_deref().map(str::trim) {
                None | Some("" | ".") => None,
                Some(p) => Some(norm(p)?),
            },
        },
        FileOp::Move { path, to_path } => FileOp::Move {
            path: norm(path)?,
            to_path: norm(to_path)?,
        },
        FileOp::Recover { id, to_path } => FileOp::Recover {
            id: id.trim().to_string(),
            to_path: match to_path {
                Some(p) => Some(norm(p)?),
                None => None,
            },
        },
    })
}

/// True for a directory on disk or one implied by files created in this batch.
fn is_dir_view(ov: &Overlay, rel: &str) -> Result<bool> {
    Ok(ov.is_dir(rel) || !ov.list_files(rel)?.is_empty())
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
