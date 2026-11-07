//! `workflow_new_turn`: closes the active turn, opens the next one and runs its first note, task
//! and file operations. The lifecycle steps are `engine::turns`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::common::params::opt_lenient_vec;
use crate::common::{AgentError, Result};
use crate::workflow::Project;
use crate::workflow::engine::items::{NoteWriteOp, TaskWriteOp, clean_title};
use crate::workflow::engine::ops::FileOp;
use crate::workflow::engine::session::model::TurnRecord;

pub const DESCRIPTION: &str = "Close the active turn and open the next one, running its first note, task and file operations. `title` cannot be changed later. At least one operation is required. The active turn is reused instead when it recorded nothing, or when `title` is given and it holds only file edits (no title, notes or tasks): it gets the title. All or nothing: if a step fails, the file changes already written are reverted and the new turn is discarded (a reused turn is restored as it was).";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NewTurnParams {
    /// Title of the new turn.
    #[serde(default)]
    pub title: Option<String>,
    /// Note operations to run in the new turn.
    #[serde(default, deserialize_with = "opt_lenient_vec")]
    #[schemars(with = "Option<Vec<NoteWriteOp>>")]
    pub write_notes: Option<Vec<NoteWriteOp>>,
    /// Handover task operations to run in the new turn.
    #[serde(default, deserialize_with = "opt_lenient_vec")]
    #[schemars(with = "Option<Vec<TaskWriteOp>>")]
    pub write_tasks: Option<Vec<TaskWriteOp>>,
    /// File operations to run in the new turn.
    #[serde(default, deserialize_with = "opt_lenient_vec")]
    #[schemars(with = "Option<Vec<FileOp>>")]
    pub write_files: Option<Vec<FileOp>>,
}

impl NewTurnParams {
    /// True when at least one of the three operation lists holds an operation.
    pub fn has_operations(&self) -> bool {
        self.write_notes.as_ref().is_some_and(|ops| !ops.is_empty())
            || self.write_tasks.as_ref().is_some_and(|ops| !ops.is_empty())
            || self.write_files.as_ref().is_some_and(|ops| !ops.is_empty())
    }
}

/// Open the next turn (settling the active one) and run the operations in it. At least one
/// note, task or file operation is required, so no empty turn is ever created. The active turn
/// is reused instead when it recorded nothing, or when `title` is given and it holds only file
/// edits, so that work started without a turn gets its title. Note and task operations are
/// validated on a copy before anything is written. If a later step still fails, the file
/// changes already written are reverted and the new turn is discarded (or the reused turn is
/// restored as it was).
pub fn new_turn(project: &mut Project, p: NewTurnParams) -> Result<Value> {
    new_turn_with_paths(project, p).map(|(out, _)| out)
}

pub(crate) fn new_turn_with_paths(
    project: &mut Project,
    p: NewTurnParams,
) -> Result<(Value, Vec<String>)> {
    if !p.has_operations() {
        return Err(AgentError::invalid(
            "write_notes/write_tasks/write_files",
            "a new turn needs at least one note, task or file operation; a title alone is not enough. Pass the turn's first changes together with `title`",
        ));
    }
    let (reuse, had_edits) = {
        let active = project.active_turn()?;
        let titled = p.title.as_deref().is_some_and(|t| clean_title(t).is_some());
        (
            active.is_empty() || (titled && active.is_file_edits_only()),
            !active.is_empty(),
        )
    };
    let closed = project.active;
    let snapshot = if reuse {
        Some(project.snapshot_active()?)
    } else {
        None
    };
    if !reuse {
        project.open_next_turn()?;
    }
    let external = match project.sync_external_edits() {
        Ok(ext) => ext,
        Err(err) => {
            project.abort_turn(closed, snapshot);
            return Err(err);
        }
    };
    // Build the turn's title, notes and tasks on a copy first, so that errors in them surface
    // before anything is changed. The copy is what gets stored later; the operations are
    // not applied a second time.
    let mut prepared = if reuse {
        match project.active_turn() {
            Ok(t) => t.clone(),
            Err(err) => {
                project.abort_turn(closed, snapshot);
                return Err(err);
            }
        }
    } else {
        TurnRecord::new(project.active)
    };
    if let Err(err) = apply_meta(project, &mut prepared, &p) {
        project.abort_turn(closed, snapshot);
        return Err(err);
    }
    match fill_turn(project, &p, prepared) {
        Ok((mut out, changed_paths)) => {
            if let Some(external) = &external {
                out["external_edits"] = external.report();
            }
            if reuse {
                out["note"] = json!(if had_edits {
                    "the active turn held only file edits, so it was titled and these changes went into it"
                } else {
                    "the active turn was empty, so these changes went into it"
                });
            } else {
                out["closed"] = json!(closed.to_string());
            }
            Ok((out, changed_paths))
        }
        Err(err) => {
            project.abort_turn(closed, snapshot);
            Err(err)
        }
    }
}

fn apply_meta(project: &Project, turn: &mut TurnRecord, p: &NewTurnParams) -> Result<()> {
    if let Some(t) = &p.title {
        turn.title = clean_title(t);
    }
    if let Some(ops) = &p.write_notes {
        project.apply_note_ops(turn, ops)?;
    }
    if let Some(ops) = &p.write_tasks {
        project.apply_task_ops(turn, ops)?;
    }
    Ok(())
}

/// Write the files in the (already opened) active turn and store `turn`, which already holds
/// the validated title, notes and tasks, together with the net file updates.
fn fill_turn(
    project: &mut Project,
    p: &NewTurnParams,
    mut turn: TurnRecord,
) -> Result<(Value, Vec<String>)> {
    let batch = match &p.write_files {
        Some(ops) if !ops.is_empty() => Some(project.apply_files(ops)?),
        _ => None,
    };
    let changed_paths = batch
        .as_ref()
        .map(|batch| batch.changed_paths.clone())
        .unwrap_or_default();
    turn.updates = project.active_turn()?.updates.clone();
    let mut out = json!({ "turn_id": turn.key.to_string() });
    if p.write_notes.is_some() {
        out["note_ids"] = json!(turn.note_ids());
    }
    if p.write_tasks.is_some() {
        out["task_ids"] = json!(
            turn.task_dtos()
                .into_iter()
                .map(|t| t.id)
                .collect::<Vec<_>>()
        );
    }
    if let Some(batch) = batch
        && let (Some(out), Value::Object(fields)) = (out.as_object_mut(), json!(batch))
    {
        out.extend(fields);
    }
    project.replace_active(turn)?;
    Ok((out, changed_paths))
}
