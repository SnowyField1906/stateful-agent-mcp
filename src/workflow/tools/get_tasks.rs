//! `workflow_get_tasks`.

use serde_json::{Value, json};

use crate::common::Result;
use crate::workflow::Project;
use crate::workflow::engine::items::GetItemsParams;
use crate::workflow::engine::selector::Sel;

pub const DESCRIPTION: &str = "Read handover tasks of past turns. `implicit_ids` takes the selectors of `workflow_get_notes`, and task IDs such as `01.02.task.01`.";

pub fn get_tasks(project: &Project, p: GetItemsParams) -> Result<Value> {
    project.items_by_selectors(
        &p.implicit_ids,
        "handover_tasks",
        |sel| match sel {
            Sel::Task(key, idx) => Ok(Some((key, idx))),
            Sel::Turns { .. } => Ok(None),
            _ => Err("a task ID, turn, session or range"),
        },
        |turn| json!(turn.task_dtos()),
        |turn, idx| turn.task_dtos().into_iter().nth(idx).map(|t| json!(t)),
    )
}
