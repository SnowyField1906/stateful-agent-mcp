//! Turn lifecycle: closing the active turn, opening the next one and running the optional
//! operations of `create_new_turn`. Notes and handover tasks live in their own modules.

use serde_json::{Value, json};

use crate::error::{AgentError, Result};
use crate::params::CreateNewTurnParams;
use crate::session::model::{TurnKey, TurnRecord};
use crate::session::store;

use super::items::clean_title;
use super::project::Project;

impl Project {
    /// Open the next turn (settling the active one) and run the operations in it. At least one
    /// note, task or file operation is required, so no empty turn is ever created. An active
    /// turn that recorded nothing is reused instead. Note and task operations are validated on a
    /// copy before anything is written. If a later step still fails, the file changes already
    /// written are reverted and the new turn is discarded (or the reused turn is left empty).
    pub fn create_new_turn(&mut self, p: CreateNewTurnParams) -> Result<Value> {
        if !p.has_operations() {
            return Err(AgentError::invalid(
                "write_notes/write_tasks/write_files",
                "a new turn needs at least one note, task or file operation; a title alone is not enough. Pass the turn's first changes together with `title`",
            ));
        }
        let closed = self.active;
        let reuse = self.active_turn()?.is_empty();
        // Build the turn's title, notes and tasks on a copy first, so that errors in them surface
        // before anything is changed. The copy is what gets stored later; the operations are
        // not applied a second time.
        let mut prepared = if reuse {
            self.active_turn()?.clone()
        } else {
            TurnRecord::new(TurnKey::new(closed.session, closed.turn + 1))
        };
        self.apply_meta(&mut prepared, &p)?;

        if !reuse {
            self.open_next_turn()?;
        }
        match self.fill_turn(&p, prepared) {
            Ok(mut out) => {
                if reuse {
                    out["note"] = json!("active turn was empty; no new turn created");
                } else {
                    out["closed"] = json!(closed.to_string());
                }
                Ok(out)
            }
            Err(err) => {
                self.rollback_active_files();
                if reuse {
                    if let Ok(turn) = self.active_turn_mut() {
                        *turn = TurnRecord::new(closed);
                    }
                    let _ = self.save_session(closed.session);
                } else {
                    self.discard_active_turn(closed);
                }
                Err(err)
            }
        }
    }

    fn apply_meta(&self, turn: &mut TurnRecord, p: &CreateNewTurnParams) -> Result<()> {
        if let Some(t) = &p.title {
            turn.title = clean_title(t);
        }
        if let Some(ops) = &p.write_notes {
            self.apply_note_ops(turn, ops)?;
        }
        if let Some(ops) = &p.write_tasks {
            self.apply_task_ops(turn, ops)?;
        }
        Ok(())
    }

    /// Write the files in the (already opened) active turn and store `turn`, which already holds
    /// the validated title, notes and tasks, together with the net file updates.
    fn fill_turn(&mut self, p: &CreateNewTurnParams, mut turn: TurnRecord) -> Result<Value> {
        let files = match &p.write_files {
            Some(ops) if !ops.is_empty() => Some(self.write_files(ops.clone())?),
            _ => None,
        };
        turn.updates = self.active_turn()?.updates.clone();
        let mut out = json!({ "turn_id": turn.key.to_string() });
        if p.write_notes.is_some() {
            out["note_ids"] = json!(
                turn.note_dtos()
                    .into_iter()
                    .map(|n| n.id)
                    .collect::<Vec<_>>()
            );
        }
        if p.write_tasks.is_some() {
            out["task_ids"] = json!(
                turn.task_dtos()
                    .into_iter()
                    .map(|t| t.id)
                    .collect::<Vec<_>>()
            );
        }
        if let Some(mut files) = files {
            if let Some(updates) = files.get_mut("updates").map(Value::take) {
                out["updates"] = updates;
            }
            if let Some(warnings) = files.get_mut("format_warnings").map(Value::take) {
                out["format_warnings"] = warnings;
            }
        }
        self.replace_active(turn)?;
        Ok(out)
    }

    fn open_next_turn(&mut self) -> Result<()> {
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

    /// Remove the active turn and make `previous` active again. Best effort: used on failure paths.
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
    /// the turn's `.deleted/` records. Best effort, used on failure paths: errors are ignored so
    /// the original error is the one reported. Directories are pruned only when a created file
    /// leaves them empty.
    fn rollback_active_files(&mut self) {
        let key = self.active;
        let baseline = std::mem::take(&mut self.active_baseline);
        self.active_moves.clear();
        for (rel, before) in &baseline {
            let path = self.root.join(rel);
            match before {
                Some(text) => {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = store::atomic_write(&path, text.as_bytes());
                }
                None => {
                    if std::fs::remove_file(&path).is_ok() {
                        store::prune_empty_dirs(&path, &self.root);
                    }
                }
            }
        }
        self.deleted.remove(&key);
        let _ = store::remove_turn_deleted(&self.deleted_dir, key);
    }

    /// Store `turn` as the active turn and persist its session.
    pub(super) fn replace_active(&mut self, turn: TurnRecord) -> Result<()> {
        *self.active_turn_mut()? = turn;
        self.save_session(self.active.session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{FileOp, PatchHunk};

    #[test]
    fn rollback_restores_files_and_forgets_deleted_records() {
        let dir =
            std::env::temp_dir().join(format!("agent-mcp-turns-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut project = Project::open(&dir.to_string_lossy()).unwrap();
        project
            .write_files(vec![
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
        project
            .create_new_turn(CreateNewTurnParams {
                title: Some("change files".into()),
                write_notes: None,
                write_tasks: None,
                write_files: Some(vec![
                    FileOp::Patch {
                        path: "a.txt".into(),
                        hunks: vec![PatchHunk {
                            start_line: None,
                            old_text: "one\n".into(),
                            new_text: "two\n".into(),
                        }],
                    },
                    FileOp::Delete {
                        path: "b.txt".into(),
                    },
                    FileOp::Create {
                        path: "sub/c.txt".into(),
                        content: "new\n".into(),
                    },
                ]),
            })
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

        let _ = std::fs::remove_dir_all(&dir);
    }
}
