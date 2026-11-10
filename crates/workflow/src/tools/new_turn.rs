//! `workflow_new_turn`: closes the active turn, opens the next one and runs its first note, task
//! and file operations. The lifecycle steps are `engine::turns`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Project;
use crate::common::Result;
use crate::common::params::opt_lenient_vec;
use crate::engine::items::{NoteWriteOp, TaskWriteOp};
use crate::engine::ops::FileOp;
use crate::engine::turns::{Opened, TurnStart};

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
    /// File operation objects to run in the new turn, as in `ops` of `workflow_write_files`.
    #[serde(default, deserialize_with = "opt_lenient_vec")]
    #[schemars(with = "Option<Vec<Value>>")]
    pub write_files: Option<Vec<FileOp>>,
}

impl NewTurnParams {
    /// What the call starts the turn with.
    fn start(&self) -> TurnStart<'_> {
        TurnStart {
            title: self.title.as_deref(),
            notes: self.write_notes.as_deref(),
            tasks: self.write_tasks.as_deref(),
            files: self.write_files.as_deref(),
        }
    }
}

/// Open the next turn (settling the active one) and run the operations in it; see
/// [`Project::start_turn`].
pub fn new_turn(project: &mut Project, p: NewTurnParams) -> Result<Value> {
    new_turn_with_paths(project, p).map(|(out, _)| out)
}

pub fn new_turn_with_paths(
    project: &mut Project,
    p: NewTurnParams,
) -> Result<(Value, Vec<String>)> {
    let started = project.start_turn(&p.start())?;
    let turn = project.active_turn()?;
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
    let changed_paths = started
        .batch
        .as_ref()
        .map(|batch| batch.changed_paths.clone())
        .unwrap_or_default();
    if let Some(batch) = &started.batch
        && let (Some(out), Value::Object(fields)) = (out.as_object_mut(), json!(batch))
    {
        out.extend(fields);
    }
    if let Some(external) = &started.external {
        out["external_edits"] = external.report();
    }
    match started.opened {
        Opened::Reused { had_edits } => {
            out["note"] = json!(if had_edits {
                "the active turn held only file edits, so it was titled and these changes went into it"
            } else {
                "the active turn was empty, so these changes went into it"
            });
        }
        Opened::Next { closed } => out["closed"] = json!(closed.to_string()),
    }
    Ok((out, changed_paths))
}
