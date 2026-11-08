//! Turn lifecycle on the [`Project`]: starting a turn from its first operations, opening the next
//! turn, storing the active one, and undoing a turn that failed while being filled.

use crate::common::io::atomic_write;
use crate::common::{AgentError, Result};

use super::Project;
use super::external::ExternalEdits;
use super::files::Batch;
use super::items::{NoteWriteOp, TaskWriteOp, clean_title};
use super::ops::overlay::read_text_opt;
use super::ops::{FileOp, Tree};
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

/// What a new turn starts with: its title and its first note, task and file operations.
pub(crate) struct TurnStart<'a> {
    pub title: Option<&'a str>,
    pub notes: Option<&'a [NoteWriteOp]>,
    pub tasks: Option<&'a [TaskWriteOp]>,
    pub files: Option<&'a [FileOp]>,
}

impl TurnStart<'_> {
    /// True when at least one of the three operation lists holds an operation.
    fn has_operations(&self) -> bool {
        self.notes.is_some_and(|ops| !ops.is_empty())
            || self.tasks.is_some_and(|ops| !ops.is_empty())
            || self.files.is_some_and(|ops| !ops.is_empty())
    }
}

/// How [`Project::start_turn`] got its turn.
pub(crate) enum Opened {
    /// A new turn was opened and `closed` was left behind.
    Next { closed: TurnKey },
    /// The active turn was reused; `had_edits` tells whether it held file edits.
    Reused { had_edits: bool },
}

/// The outcome of [`Project::start_turn`] besides the stored turn itself, which is the active one.
pub(crate) struct StartedTurn {
    pub opened: Opened,
    pub external: Option<ExternalEdits>,
    pub batch: Option<Batch>,
}

impl Project {
    /// Open the next turn (settling the active one) and run `start` in it. At least one note, task
    /// or file operation is required, so no empty turn is ever created. The active turn is reused
    /// instead when it recorded nothing, or when the title is given and it holds only file edits,
    /// so that work started without a turn gets its title. Note and task operations are validated
    /// on a copy before any file is written. If a later step still fails, the file changes already
    /// written are reverted and the new turn is discarded (or the reused turn is restored as it
    /// was).
    pub(crate) fn start_turn(&mut self, start: &TurnStart<'_>) -> Result<StartedTurn> {
        if !start.has_operations() {
            return Err(AgentError::invalid(
                "write_notes/write_tasks/write_files",
                "a new turn needs at least one note, task or file operation; a title alone is not enough. Pass the turn's first changes together with `title`",
            ));
        }
        let active = self.active_turn()?;
        let titled = start.title.is_some_and(|t| clean_title(t).is_some());
        let reuse = active.is_empty() || (titled && active.is_file_edits_only());
        let had_edits = !active.is_empty();
        let closed = self.active;
        let snapshot = if reuse {
            Some(self.snapshot_active()?)
        } else {
            self.open_next_turn()?;
            None
        };
        match self.fill_started_turn(start, reuse) {
            Ok((external, batch)) => Ok(StartedTurn {
                opened: if reuse {
                    Opened::Reused { had_edits }
                } else {
                    Opened::Next { closed }
                },
                external,
                batch,
            }),
            Err(err) => {
                self.abort_turn(closed, snapshot);
                Err(err)
            }
        }
    }

    /// Record outside edits, build the turn's title, notes and tasks on a copy, write the files and
    /// store the copy together with the net file updates.
    fn fill_started_turn(
        &mut self,
        start: &TurnStart<'_>,
        reuse: bool,
    ) -> Result<(Option<ExternalEdits>, Option<Batch>)> {
        let external = self.sync_external_edits()?;
        let mut turn = if reuse {
            self.active_turn()?.clone()
        } else {
            TurnRecord::new(self.active)
        };
        if let Some(title) = start.title {
            turn.title = clean_title(title);
        }
        if let Some(ops) = start.notes {
            self.apply_note_ops(&mut turn, ops)?;
        }
        if let Some(ops) = start.tasks {
            self.apply_task_ops(&mut turn, ops)?;
        }
        let batch = match start.files {
            Some(ops) if !ops.is_empty() => Some(self.apply_files(ops)?),
            _ => None,
        };
        turn.updates = self.active_turn()?.updates.clone();
        self.replace_active(turn)?;
        Ok((external, batch))
    }

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
