//! Turn lifecycle on the [`Project`]: opening the next turn, storing the active one, and undoing a
//! turn that failed while being filled.

use crate::common::io::atomic_write;
use crate::common::{AgentError, Result};

use super::Project;
use super::ops::Tree;
use super::ops::overlay::read_text_opt;
use super::session::model::{TitleChange, TurnKey, TurnRecord};
use super::session::store::{self, DeletedMap};

/// The active turn as it was before a call that reuses it, to put back if the call fails.
pub(crate) struct TurnSnapshot {
    turn: TurnRecord,
    /// Content of every file the turn had touched, as it was on disk; `None` where the turn
    /// had deleted it.
    files: Tree,
    deleted: Option<DeletedMap>,
}

impl Project {
    /// Open the turn after the active one and make it active.
    pub(crate) fn open_next_turn(&mut self) -> Result<()> {
        let previous = self.active;
        let key = TurnKey::new(previous.session, previous.turn + 1);
        self.sessions
            .get_mut(&previous.session)
            .ok_or_else(|| AgentError::NotFound(format!("session {:02}", previous.session)))?
            .push(TurnRecord::new(key));
        self.active = key;
        self.active_baseline.clear();
        self.active_moves.clear();
        if let Err(err) = self.save_session(key.session) {
            self.discard_active_turn(previous);
            return Err(err);
        }
        Ok(())
    }

    /// Store `turn` as the active turn and persist its session.
    pub(crate) fn replace_active(&mut self, turn: TurnRecord) -> Result<()> {
        *self.active_turn_mut()? = turn;
        self.save_session(self.active.session)
    }

    /// Apply the `title` of a `write_*` call to the active turn and persist the session when the
    /// turn took it. Call this only after the call's other changes succeeded, so that a failed
    /// call does not name the turn.
    pub(crate) fn name_active_turn(&mut self, title: Option<&str>) -> Result<TitleChange> {
        let change = self.active_turn_mut()?.adopt_title(title);
        if change == TitleChange::Named {
            self.save_session(self.active.session)?;
        }
        Ok(change)
    }

    /// The active turn as it is now, to restore when a call that reuses it fails.
    pub(crate) fn snapshot_active(&self) -> Result<TurnSnapshot> {
        let mut files = Tree::new();
        for path in self.active_baseline.keys() {
            files.insert(path.clone(), read_text_opt(&self.root, path)?);
        }
        Ok(TurnSnapshot {
            turn: self.active_turn()?.clone(),
            files,
            deleted: self.deleted.get(&self.active).cloned(),
        })
    }

    /// Undo a call that failed while filling a turn. A turn the call opened is dropped after its
    /// files are put back; a reused turn is restored from `snapshot`, with the edits it held before
    /// the call. Best effort: errors are ignored so the caller's error is the one reported.
    pub(crate) fn abort_turn(&mut self, previous: TurnKey, snapshot: Option<TurnSnapshot>) {
        match snapshot {
            Some(snapshot) => self.restore_active(snapshot),
            None => {
                self.rollback_active_files();
                self.discard_active_turn(previous);
            }
        }
    }

    /// Put the active turn back as `snapshot` recorded it: its files, `.deleted/` records, record
    /// and the baseline derived from them.
    fn restore_active(&mut self, snapshot: TurnSnapshot) {
        let key = self.active;
        let baseline = std::mem::take(&mut self.active_baseline);
        self.active_moves.clear();
        // A file the call touched first is back to its pre-turn content; one the turn already
        // touched is back to what it held before the call.
        for (rel, before) in &baseline {
            let content = snapshot.files.get(rel).unwrap_or(before);
            self.restore_file(rel, content.as_deref());
        }
        match snapshot.deleted {
            Some(map) => {
                let _ =
                    store::sync_turn_deleted(&self.deleted_dir, key, self.deleted.get(&key), &map);
                self.deleted.insert(key, map);
            }
            None => {
                self.deleted.remove(&key);
                let _ = store::remove_turn_deleted(&self.deleted_dir, key);
            }
        }
        if let Ok(turn) = self.active_turn_mut() {
            *turn = snapshot.turn;
        }
        let _ = self.save_session(key.session);
        let _ = self.init_active_state();
    }

    /// Write `content` to `rel`, or remove the file (and directories it leaves empty) for `None`.
    fn restore_file(&mut self, rel: &str, content: Option<&str>) {
        let path = self.root.join(rel);
        match content {
            Some(text) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = atomic_write(&path, text.as_bytes());
                let (size, mtime) = if let Ok(meta) = std::fs::symlink_metadata(&path) {
                    (meta.len(), store::meta_mtime(&meta))
                } else {
                    (text.len() as u64, 0)
                };
                let hash = store::content_hash(text.as_bytes());
                self.known_index
                    .insert(rel.to_string(), store::KnownEntry { size, mtime, hash });
            }
            None => {
                if std::fs::remove_file(&path).is_ok() {
                    store::prune_empty_dirs(&path, &self.root);
                }
                self.known_index.remove(rel);
            }
        }
        // The file holds again what the ledger last saw of it.
        let _ = store::write_known(&self.known_dir, rel, content);
        let _ = self.save_known_index();
    }

    /// Remove the active turn and make `previous` active again.
    fn discard_active_turn(&mut self, previous: TurnKey) {
        let key = self.active;
        if let Some(turns) = self.sessions.get_mut(&key.session)
            && turns.last().is_some_and(|t| t.key == key)
        {
            turns.pop();
        }
        self.deleted.remove(&key);
        let _ = store::remove_turn_deleted(&self.deleted_dir, key);
        self.active = previous;
        let _ = self.init_active_state();
        let _ = self.save_session(key.session);
    }

    /// Put every file touched in the active turn back to its content before the turn, and drop
    /// the turn's `.deleted/` records. Directories are pruned only when a created file leaves
    /// them empty.
    fn rollback_active_files(&mut self) {
        let key = self.active;
        let baseline = std::mem::take(&mut self.active_baseline);
        self.active_moves.clear();
        for (rel, before) in &baseline {
            self.restore_file(rel, before.as_deref());
        }
        self.deleted.remove(&key);
        let _ = store::remove_turn_deleted(&self.deleted_dir, key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::engine::ops::{FileOp, PatchHunk};

    #[test]
    fn rollback_restores_files_and_forgets_deleted_records() {
        let dir =
            std::env::temp_dir().join(format!("agent-mcp-turns-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut project = Project::open(&dir.to_string_lossy()).unwrap();
        project
            .apply_files(&[
                FileOp::Create {
                    path: "a.txt".into(),
                    content: "one\n".into(),
                },
                FileOp::Create {
                    path: "b.txt".into(),
                    content: "keep\n".into(),
                },
            ])
            .unwrap();
        project.open_next_turn().unwrap();
        project
            .apply_files(&[
                FileOp::Patch {
                    path: "a.txt".into(),
                    hunks: vec![PatchHunk {
                        start_line: None,
                        old_text: "one\n".into(),
                        new_text: "two\n".into(),
                        symbol: None,
                    }],
                },
                FileOp::Delete {
                    path: "b.txt".into(),
                },
                FileOp::Create {
                    path: "sub/c.txt".into(),
                    content: "new\n".into(),
                },
            ])
            .unwrap();
        let key = project.active;
        assert!(project.deleted.contains_key(&key));
        assert!(dir.join(".AGENTS/.deleted").join(key.to_string()).exists());

        project.rollback_active_files();

        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\n");
        assert_eq!(
            std::fs::read_to_string(dir.join("b.txt")).unwrap(),
            "keep\n"
        );
        assert!(!dir.join("sub").exists());
        assert!(!project.deleted.contains_key(&key));
        assert!(!dir.join(".AGENTS/.deleted").join(key.to_string()).exists());
        assert!(project.active_baseline.is_empty());
        // The copies of what the ledger last saw follow the rollback.
        let known = dir.join(".AGENTS/.known");
        assert_eq!(
            std::fs::read_to_string(known.join("a.txt")).unwrap(),
            "one\n"
        );
        assert_eq!(
            std::fs::read_to_string(known.join("b.txt")).unwrap(),
            "keep\n"
        );
        assert!(!known.join("sub").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn abort_of_a_reused_turn_keeps_the_edits_made_before_the_call() {
        let dir =
            std::env::temp_dir().join(format!("agent-mcp-turns-restore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut project = Project::open(&dir.to_string_lossy()).unwrap();
        let create = |path: &str, content: &str| FileOp::Create {
            path: path.into(),
            content: content.into(),
        };
        project.apply_files(&[create("a.txt", "one\n")]).unwrap();
        let snapshot = project.snapshot_active().unwrap();
        let updates_before = project.active_turn().unwrap().updates.clone();

        project
            .apply_files(&[
                FileOp::Patch {
                    path: "a.txt".into(),
                    hunks: vec![PatchHunk {
                        start_line: None,
                        old_text: "one\n".into(),
                        new_text: "two\n".into(),
                        symbol: None,
                    }],
                },
                create("b.txt", "new\n"),
            ])
            .unwrap();
        let previous = project.active;
        project.abort_turn(previous, Some(snapshot));

        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\n");
        assert!(!dir.join("b.txt").exists());
        assert_eq!(project.active_turn().unwrap().updates, updates_before);
        assert_eq!(
            project.active_baseline.keys().collect::<Vec<_>>(),
            vec!["a.txt"]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
