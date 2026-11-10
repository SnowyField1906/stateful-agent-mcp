//! Edits made to tracked files outside the ledger.
//!
//! The ledger rebuilds past file contents by reverse-applying turn hunks from `.AGENTS/.known/`,
//! which keeps, for every file the ledger has touched, the content it last saw. That chain stays
//! exact while the file is left alone, so a change made to a file outside the ledger costs
//! nothing until a batch is about to change the same file. Then, and only for the files of that
//! batch, the file on disk is compared with its copy, before the batch is committed:
//!
//! - a change that is only the project's formatter at work is merged into the turn that last
//!   changed the file, so that no turn records it;
//! - any other change is recorded as a turn of its own, titled [`EXTERNAL_EDIT_TITLE`], ahead of
//!   the turn that follows. That turn starts from the edited content, so its diff holds only what
//!   the agent did. When a formatter was involved, the turn holds only the change that is left
//!   once the formatter's share is merged.
//!
//! Files that the batch leaves alone are not compared: a human's edits, a formatter run over the
//! whole project or a linter's fixes in files the agent has no business with leave no trace in
//! the ledger.
//!
//! A file the active turn has already touched is left alone: the turn's net diff, from its
//! baseline to the file on disk, already includes the edit.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::common::Result;
use crate::toolchain::LanguageConfig;
use crate::toolchain::format::format_text;

use super::Project;
use super::ops::diff::compute_hunks;
use super::ops::overlay::read_text_opt;
use super::ops::{FileOp, Tree};
use super::session::model::{EXTERNAL_EDIT_TITLE, TurnKey};
use super::session::store::{self, DeletedMap};

/// What was found changed on disk in the files of a batch.
pub(crate) struct ExternalEdits {
    /// The turn that records the changes; `None` when they were all the formatter's.
    pub turn: Option<TurnKey>,
    /// The files that turn records.
    pub paths: Vec<String>,
    /// Files whose formatter-made changes were merged into the turn that last changed them.
    pub formatted: BTreeMap<String, TurnKey>,
}

impl ExternalEdits {
    /// The `external_edits` field of a tool response.
    pub(crate) fn report(&self) -> Value {
        let mut out = Map::new();
        if let Some(turn) = self.turn {
            out.insert("turn_id".into(), json!(turn.to_string()));
            out.insert("paths".into(), json!(self.paths));
            out.insert(
                "note".into(),
                json!(
                    "these files changed on disk outside the ledger; the changes are recorded in this turn and later turns start from them"
                ),
            );
        }
        if !self.formatted.is_empty() {
            let merged: Map<String, Value> = self
                .formatted
                .iter()
                .map(|(path, key)| (path.clone(), json!(key.to_string())))
                .collect();
            out.insert("formatting_merged".into(), Value::Object(merged));
        }
        Value::Object(out)
    }
}

impl Project {
    /// Settle the changes made outside the ledger to `touched`, the files a batch is about to
    /// change. Formatter-made changes are merged into the turn that last changed each file; the
    /// rest become a closed turn of their own (the active turn when it is still empty) and a new
    /// active turn is opened after it. Returns `None` when nothing changed.
    pub(crate) fn sync_external_edits(
        &mut self,
        touched: &BTreeSet<String>,
    ) -> Result<Option<ExternalEdits>> {
        self.adopt_untracked_files(touched)?;
        let config = self.language_config().unwrap_or_default();
        let flat = self.flat_keys();

        let mut updates = Vec::new();
        let mut recorded = Vec::new();
        let mut deleted = DeletedMap::new();
        let mut now = Tree::new();
        let mut amended: BTreeMap<TurnKey, Vec<FileOp>> = BTreeMap::new();
        let mut formatted = BTreeMap::new();
        let mut index_refreshes = Vec::new();

        for rel in touched {
            if !self.known_index.contains_key(rel) {
                continue;
            }
            let meta_opt = std::fs::symlink_metadata(self.root.join(rel)).ok();
            if let Some(meta) = &meta_opt {
                if !meta.is_file() {
                    continue;
                }
                if let Some(entry) = self.known_index.get(rel)
                    && entry.size == meta.len()
                    && entry.mtime == store::meta_mtime(meta)
                {
                    // Metadata matches perfectly. File content definitely unchanged!
                    continue;
                }
            }

            let Ok(disk) = read_text_opt(&self.root, rel) else {
                continue;
            };
            let Some(before) = store::read_known(&self.known_dir, rel)? else {
                continue;
            };

            if disk.as_deref() == Some(before.as_str()) {
                if let Some(meta) = &meta_opt {
                    let hash = store::content_hash(before.as_bytes());
                    let entry = store::KnownEntry {
                        size: meta.len(),
                        mtime: store::meta_mtime(meta),
                        hash,
                    };
                    index_refreshes.push((rel.clone(), entry));
                }
                continue;
            }

            if disk.is_some() && self.active_baseline.contains_key(rel) {
                continue;
            }

            match &disk {
                Some(text) => {
                    // The change is measured from the formatter's version of the old content when
                    // the formatter accounts for part of it and a turn can take that part.
                    let mut start = before.clone();
                    if let Some(share) = formatter_share(&config, &self.root, rel, &before, text)
                        && let Some((key, ops)) =
                            self.merge_formatting(&flat, rel, &share, &amended)
                    {
                        amended.insert(key, ops);
                        formatted.insert(rel.clone(), key);
                        start = share;
                    }
                    if start != *text {
                        updates.push(FileOp::Patch {
                            path: rel.clone(),
                            hunks: compute_hunks(&start, text),
                        });
                        recorded.push(rel.clone());
                    }
                }
                None => {
                    updates.push(FileOp::Delete { path: rel.clone() });
                    recorded.push(rel.clone());
                    deleted.insert(rel.clone(), before);
                }
            }
            now.insert(rel.clone(), disk);
        }

        for (rel, entry) in index_refreshes {
            self.known_index.insert(rel, entry);
        }
        let _ = self.save_known_index();

        if updates.is_empty() && formatted.is_empty() {
            return Ok(None);
        }

        let mut saved = BTreeSet::new();
        for (key, ops) in amended {
            if let Some(turn) = self
                .sessions
                .get_mut(&key.session)
                .and_then(|turns| turns.iter_mut().find(|t| t.key == key))
            {
                turn.updates = ops;
                saved.insert(key.session);
            }
        }
        for session in saved {
            self.save_session(session)?;
        }

        let mut external_turn = None;
        if !updates.is_empty() {
            if !self.active_turn()?.is_empty() {
                self.open_next_turn()?;
            }
            let key = self.active;
            let record = self.active_turn_mut()?;
            record.title = Some(EXTERNAL_EDIT_TITLE.to_string());
            record.updates = updates;
            if !deleted.is_empty() {
                store::sync_turn_deleted(&self.deleted_dir, key, None, &deleted)?;
                self.deleted.insert(key, deleted);
            }
            self.save_session(key.session)?;
            self.open_next_turn()?;
            external_turn = Some(key);
        }
        self.record_known(&now)?;
        Ok(Some(ExternalEdits {
            turn: external_turn,
            paths: recorded,
            formatted,
        }))
    }

    /// The record of the turn that last changed the tracked file `rel`, with the file ending in
    /// `formatted` (the formatter's version of what that turn left), or `None` when no turn can
    /// take it: the turn moved or deleted the file, holds several operations on it, or the chain
    /// of recorded content does not lead to it. `amended` holds records already changed by this
    /// sync, which concern other files.
    fn merge_formatting(
        &self,
        flat: &[TurnKey],
        rel: &str,
        formatted: &str,
        amended: &BTreeMap<TurnKey, Vec<FileOp>>,
    ) -> Option<(TurnKey, Vec<FileOp>)> {
        let key = self.last_key_touching(rel)?;
        let at = flat.iter().position(|k| *k == key)?;
        let recorded = match amended.get(&key) {
            Some(ops) => ops.as_slice(),
            None => self.turn(key)?.updates.as_slice(),
        };
        let before = self.content_before(flat, at, rel).ok()?;
        let ops = with_final_text(recorded, rel, before.as_deref(), formatted)?;
        Some((key, ops))
    }

    /// Remember `files` as what the ledger last saw of them; `None` is a file that is gone.
    /// Removals come first, so a path that turns from a file into a directory finds its place
    /// free. Whatever changes tracked files on disk outside a batch has to call this too.
    pub(crate) fn record_known(&mut self, files: &Tree) -> Result<()> {
        for rel in files
            .iter()
            .filter(|(_, content)| content.is_none())
            .map(|(rel, _)| rel)
        {
            store::write_known(&self.known_dir, rel, None)?;
            self.known_index.remove(rel);
        }
        for (rel, content) in files {
            if let Some(text) = content {
                store::write_known(&self.known_dir, rel, Some(text))?;
                let disk_path = self.root.join(rel);
                let (size, mtime) = if let Ok(meta) = std::fs::symlink_metadata(&disk_path) {
                    (meta.len(), store::meta_mtime(&meta))
                } else {
                    (text.len() as u64, 0)
                };
                let hash = store::content_hash(text.as_bytes());
                self.known_index
                    .insert(rel.clone(), store::KnownEntry { size, mtime, hash });
            }
        }
        self.save_known_index()?;
        Ok(())
    }

    /// Start keeping a copy of those `touched` files that the recorded turns leave in place and
    /// that have none yet, as they are on disk: a ledger from before copies were kept has no
    /// earlier state to compare with.
    fn adopt_untracked_files(&mut self, touched: &BTreeSet<String>) -> Result<()> {
        let unknown: Vec<&String> = touched
            .iter()
            .filter(|rel| !self.known_index.contains_key(*rel))
            .collect();
        if unknown.is_empty() {
            return Ok(());
        }
        let left = self.files_left_by_turns();
        let mut adopted = Tree::new();
        for rel in unknown {
            if left.contains(rel)
                && let Ok(Some(text)) = read_text_opt(&self.root, rel)
            {
                adopted.insert(rel.clone(), Some(text));
            }
        }
        if adopted.is_empty() {
            return Ok(());
        }
        self.record_known(&adopted)
    }

    /// The files that exist after the last recorded turn, going by the turns' updates.
    fn files_left_by_turns(&self) -> BTreeSet<String> {
        let mut files = BTreeSet::new();
        for key in self.flat_keys() {
            let Some(turn) = self.turn(key) else {
                continue;
            };
            for op in &turn.updates {
                match op {
                    FileOp::Create { path, .. } | FileOp::Patch { path, .. } => {
                        files.insert(path.clone());
                    }
                    FileOp::Delete { path } => {
                        files.remove(path);
                    }
                    FileOp::Move { path, to_path } => {
                        files.remove(path);
                        files.insert(to_path.clone());
                    }
                    _ => {}
                }
            }
        }
        files
    }
}

/// The formatter's version of `before`, when the formatter accounts for the change from `before`
/// to `disk`: the disk holds that version, or an edit of it that is itself formatted. `None` when
/// `before` is formatted already (the whole change is someone's edit), when no formatter applies,
/// or when `disk` is not formatter output (a hand edit, so none of it is the formatter's).
fn formatter_share(
    config: &LanguageConfig,
    root: &Path,
    rel: &str,
    before: &str,
    disk: &str,
) -> Option<String> {
    let formatted = format_text(config, root, rel, before)?;
    if formatted == before {
        return None;
    }
    if formatted != disk && format_text(config, root, rel, disk)? != disk {
        return None;
    }
    Some(formatted)
}

/// `recorded` (the updates of one turn) with the file `path` ending in `text`. `before` is the
/// content of the file before the turn. `None` when the turn's record of the file cannot take it:
/// the turn moved or deleted the file, or holds more than one operation on it.
fn with_final_text(
    recorded: &[FileOp],
    path: &str,
    before: Option<&str>,
    text: &str,
) -> Option<Vec<FileOp>> {
    let mut ops = Vec::with_capacity(recorded.len());
    let mut found = false;
    for op in recorded {
        if !op.touches_path(path) {
            ops.push(op.clone());
            continue;
        }
        if found {
            return None;
        }
        found = true;
        match op {
            FileOp::Create { .. } if before.is_none() => ops.push(FileOp::Create {
                path: path.to_string(),
                content: text.to_string(),
            }),
            FileOp::Patch { .. } => {
                let hunks = compute_hunks(before?, text);
                if !hunks.is_empty() {
                    ops.push(FileOp::Patch {
                        path: path.to_string(),
                        hunks,
                    });
                }
            }
            _ => return None,
        }
    }
    found.then_some(ops)
}
