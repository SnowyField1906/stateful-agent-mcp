//! `workflow_write_tasks`: handover tasks of the active turn. The operations are `engine::tasks`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::common::Result;
use crate::common::params::lenient_vec;
use crate::workflow::Project;
use crate::workflow::engine::items::TaskWriteOp;
use crate::workflow::engine::session::model::TurnRecord;

pub const DESCRIPTION: &str = "Edit the active turn's handover tasks with `ops` (`Append`, `Edit`, `Delete`, `RollOver`). Handover tasks are short-term work for the next turn or session, not a project backlog. Set `user_explicit` on tasks the user asked for. `RollOver` carries an unfinished task of an earlier turn forward without retyping it. IDs in one call use the numbering from before the call. Optional `title` names the active turn when it has none; a titled turn keeps its title.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteTasksParams {
    #[serde(deserialize_with = "lenient_vec")]
    #[schemars(with = "Vec<TaskWriteOp>")]
    pub ops: Vec<TaskWriteOp>,
    /// Title for the active turn if it has none (single line). A titled turn keeps its title.
    #[serde(default)]
    pub title: Option<String>,
}

pub fn write_tasks(project: &mut Project, p: WriteTasksParams) -> Result<Value> {
    let mut turn = project.active_turn()?.clone();
    project.apply_task_ops(&mut turn, &p.ops)?;
    let title = turn.adopt_title(p.title.as_deref());
    let renumbered = p
        .ops
        .iter()
        .any(|op| matches!(op, TaskWriteOp::Delete { .. }));
    let mut out = tasks_json(&turn, renumbered);
    title.report(&turn, &mut out);
    project.replace_active(turn)?;
    Ok(out)
}

fn tasks_json(turn: &TurnRecord, renumbered: bool) -> Value {
    let mut out = json!({
        "turn_id": turn.key.to_string(),
        "task_ids": turn.task_dtos().into_iter().map(|t| t.id).collect::<Vec<_>>(),
    });
    if renumbered {
        out["renumbered"] = json!("task_ids lists the tasks that remain, numbered from 01");
    }
    out
}
