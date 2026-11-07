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
        let known = store::known_paths(&self.known_dir)?;
        self.adopt_untracked_files(&known)?;

        let mut updates = Vec::new();
        let mut deleted = DeletedMap::new();
        let mut now = Tree::new();
        for rel in known {
            let Some(before) = store::read_known(&self.known_dir, &rel)? else {
                continue;
            };
            // A directory or a binary file at the path is not text the ledger can follow.
            let Ok(disk) = read_text_opt(&self.root, &rel) else {
                continue;
            };
            if disk.as_deref() == Some(before.as_str()) {
                continue;
            }
            if disk.is_some() && self.active_baseline.contains_key(&rel) {
                continue;
            }
            match &disk {
                Some(text) => updates.push(FileOp::Patch {
                    path: rel.clone(),
                    hunks: compute_hunks(&before, text),
                }),
                None => {
                    updates.push(FileOp::Delete { path: rel.clone() });
                    deleted.insert(rel.clone(), before);
                }
            }
            now.insert(rel, disk);
        }
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
    pub(crate) fn record_known(&self, files: &Tree) -> Result<()> {
        for rel in files
            .iter()
            .filter(|(_, content)| content.is_none())
            .map(|(rel, _)| rel)
        {
            store::write_known(&self.known_dir, rel, None)?;
        }
        for (rel, content) in files {
            if let Some(text) = content {
                store::write_known(&self.known_dir, rel, Some(text))?;
            }
        }
        Ok(())
    }

    /// Start keeping a copy of the files the recorded turns leave in place that have none yet,
    /// as they are on disk: a ledger from before copies were kept has no earlier state to
    /// compare with.
    fn adopt_untracked_files(&self, known: &[String]) -> Result<()> {
        let known: BTreeSet<&str> = known.iter().map(String::as_str).collect();
        for rel in self.files_left_by_turns() {
            if known.contains(rel.as_str()) {
                continue;
            }
            if let Ok(Some(text)) = read_text_opt(&self.root, &rel) {
                store::write_known(&self.known_dir, &rel, Some(&text))?;
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
