//! Edits made to tracked files outside the ledger.
//!
//! The ledger rebuilds past file contents by reverse-applying turn hunks from the files on
//! disk, so each turn has to begin with the content the previous turn left. `.AGENTS/.known/`
//! keeps, for every file the ledger has touched, the content it last saw. When a project is
//! registered and before each write, the files are compared with those copies. A file that
//! differs was changed by someone else: the change is recorded as a turn of its own, titled
//! [`EXTERNAL_EDIT_TITLE`], ahead of the turn that follows. That turn starts from the edited
//! content, so its diff holds only what the agent did.
//!
//! A file the active turn has already touched is left alone: the turn's net diff, from its
//! baseline to the file on disk, already includes the edit.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::common::Result;

use super::Project;
use super::ops::diff::compute_hunks;
use super::ops::overlay::read_text_opt;
use super::ops::{FileOp, Tree};
use super::session::model::{EXTERNAL_EDIT_TITLE, TurnKey};
use super::session::store::{self, DeletedMap};

/// The files found changed on disk and the turn that records them.
pub(crate) struct ExternalEdits {
    pub turn: TurnKey,
    pub paths: Vec<String>,
}

impl ExternalEdits {
    /// The `external_edits` field of a tool response.
    pub(crate) fn report(&self) -> Value {
        json!({
            "turn_id": self.turn.to_string(),
            "paths": self.paths,
            "note": "these files changed on disk outside the ledger; the changes are recorded in this turn and later turns start from them",
        })
    }
}

impl Project {
    /// Record the changes made to tracked files since the ledger last saw them. The changes
    /// become a closed turn of their own (the active turn when it is still empty) and a new
    /// active turn is opened after it. Returns `None` when nothing changed.
    pub(crate) fn sync_external_edits(&mut self) -> Result<Option<ExternalEdits>> {
        let known: Vec<String> = self.known_index.keys().cloned().collect();
        self.adopt_untracked_files(&known)?;

        let mut updates = Vec::new();
        let mut deleted = DeletedMap::new();
        let mut now = Tree::new();
        let mut index_refreshes = Vec::new();

        let candidates: Vec<String> = self.known_index.keys().cloned().collect();
        for rel in candidates {
            let disk_path = self.root.join(&rel);
            let meta_opt = std::fs::symlink_metadata(&disk_path).ok();

            if let Some(meta) = &meta_opt {
                if !meta.is_file() {
                    continue;
                }
                let size = meta.len();
                let mtime = store::meta_mtime(meta);
                if let Some(entry) = self.known_index.get(&rel) {
                    if entry.size == size && entry.mtime == mtime {
                        // Metadata matches perfectly. File content definitely unchanged!
                        continue;
                    }
                }
            }

            let Ok(disk) = read_text_opt(&self.root, &rel) else {
                continue;
            };

            let Some(before) = store::read_known(&self.known_dir, &rel)? else {
                continue;
            };

            if disk.as_deref() == Some(before.as_str()) {
                if let Some(meta) = &meta_opt {
                    let size = meta.len();
                    let mtime = store::meta_mtime(meta);
                    let hash = store::content_hash(before.as_bytes());
                    index_refreshes.push((rel, Some(store::KnownEntry { size, mtime, hash })));
                }
                continue;
            }

            if disk.is_some() && self.active_baseline.contains_key(&rel) {
                continue;
            }

            match &disk {
                Some(text) => {
                    updates.push(FileOp::Patch {
                        path: rel.clone(),
                        hunks: compute_hunks(&before, text),
                    });
                    if let Some(meta) = &meta_opt {
                        let size = meta.len();
                        let mtime = store::meta_mtime(meta);
                        let hash = store::content_hash(text.as_bytes());
                        index_refreshes
                            .push((rel.clone(), Some(store::KnownEntry { size, mtime, hash })));
                    }
                }
                None => {
                    updates.push(FileOp::Delete { path: rel.clone() });
                    deleted.insert(rel.clone(), before);
                    index_refreshes.push((rel.clone(), None));
                }
            }
            now.insert(rel, disk);
        }

        for (rel, entry) in index_refreshes {
            match entry {
                Some(e) => {
                    self.known_index.insert(rel, e);
                }
                None => {
                    self.known_index.remove(&rel);
                }
            }
        }
        let _ = self.save_known_index();

        if updates.is_empty() {
            return Ok(None);
        }
        let paths = now.keys().cloned().collect();

        if !self.active_turn()?.is_empty() {
            self.open_next_turn()?;
        }
        let key = self.active;
        let turn = self.active_turn_mut()?;
        turn.title = Some(EXTERNAL_EDIT_TITLE.to_string());
        turn.updates = updates;
        if !deleted.is_empty() {
            store::sync_turn_deleted(&self.deleted_dir, key, None, &deleted)?;
            self.deleted.insert(key, deleted);
        }
        self.save_session(key.session)?;
        self.open_next_turn()?;
        self.record_known(&now)?;
        Ok(Some(ExternalEdits { turn: key, paths }))
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

    /// Start keeping a copy of the files the recorded turns leave in place that have none yet,
    /// as they are on disk: a ledger from before copies were kept has no earlier state to
    /// compare with.
    fn adopt_untracked_files(&mut self, known: &[String]) -> Result<()> {
        let known: BTreeSet<&str> = known.iter().map(String::as_str).collect();
        for rel in self.files_left_by_turns() {
            if known.contains(rel.as_str()) {
                continue;
            }
            if let Ok(Some(text)) = read_text_opt(&self.root, &rel) {
                store::write_known(&self.known_dir, &rel, Some(&text))?;
                let disk_path = self.root.join(&rel);
                let (size, mtime) = if let Ok(meta) = std::fs::symlink_metadata(&disk_path) {
                    (meta.len(), store::meta_mtime(&meta))
                } else {
                    (text.len() as u64, 0)
                };
                let hash = store::content_hash(text.as_bytes());
                self.known_index
                    .insert(rel, store::KnownEntry { size, mtime, hash });
            }
        }
        Ok(())
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
