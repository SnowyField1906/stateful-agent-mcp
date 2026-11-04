//! `write_files`: applies a batch of file operations atomically and records the turn's net
//! changes.
//!
//! A batch is applied to an in-memory [`Overlay`]; the disk is touched only when every
//! operation succeeded. For the active turn the project keeps `active_baseline` (the content of
//! each touched file before the turn first touched it) and `active_moves` (`to_path -> from_path`).
//! After every batch the turn's `updates` are recomputed as the net difference between that
//! baseline and the files on disk, and `.AGENTS/.deleted/` is synced with any net-deleted files.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Value, json};

use crate::error::{AgentError, Result};
use crate::files::Tree;
use crate::files::net_ops::net_ops_with_moves;
use crate::files::overlay::{Overlay, read_text_opt};
use crate::files::patch::apply_hunks;
use crate::files::path::{matches_pathspec, normalize_rel};
use crate::files::summary::op_summary;
use crate::lang::format::{FormatReport, format_paths};
use crate::params::FileOp;
use crate::session::model::TurnKey;
use crate::session::store::{self, DeletedMap};

use super::project::Project;
use super::selector::Sel;

#[derive(Default)]
struct BatchTracker {
    /// File-level moves `(from_path, to_path)` performed in this batch.
    moves: Vec<(String, String)>,
    /// Paths deleted in this batch.
    deleted: Vec<String>,
}

impl Project {
    pub fn write_files(&mut self, ops: Vec<FileOp>) -> Result<Value> {
        if ops.is_empty() {
            return Err(AgentError::invalid(
                "ops",
                "at least one operation is required",
            ));
        }
        let flat = self.flat_keys();
        let mut overlay = Overlay::new(self.root.clone());
        let mut tracker = BatchTracker::default();
        let mut results = Vec::with_capacity(ops.len());
        for (i, raw) in ops.iter().enumerate() {
            let applied = normalize_op(raw, &self.root)
                .and_then(|op| self.apply_op(&mut overlay, &mut tracker, &flat, &op));
            results.push(applied.map_err(|e| in_op(i, e))?);
        }

        let key = self.active;
        if overlay.changed().next().is_none() {
            return Ok(json!({ "turn_id": key.to_string(), "changed": false, "updates": results }));
        }
        if let Err(err) = overlay.commit() {
            overlay.revert();
            return Err(err);
        }
        // Formatters rewrite files on disk; the net diff below then records the formatted text.
        let format_report = self.format_written(&overlay);

        for (path, entry) in overlay.entries() {
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

        let mut out = json!({ "turn_id": key.to_string(), "updates": results });
        if let Some(obj) = out.as_object_mut()
            && !format_report.warnings.is_empty()
        {
            obj.insert("format_warnings".into(), json!(format_report.warnings));
        }
        Ok(out)
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
    /// sync `.AGENTS/.deleted/`, and save the session. Returns the number of updates.
    pub(crate) fn refresh_active_updates(&mut self) -> Result<usize> {
        let key = self.active;
        let mut current = Tree::new();
        for path in self.active_baseline.keys() {
            current.insert(path.clone(), read_text_opt(&self.root, path)?);
        }
        let updates = net_ops_with_moves(&self.active_baseline, &current, &self.active_moves);

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

        let turn_files = updates.len();
        self.active_turn_mut()?.updates = updates;
        self.save_session(key.session)?;
        Ok(turn_files)
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
