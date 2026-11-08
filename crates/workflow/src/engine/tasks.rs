//! Applying handover-task operations to a turn.

use crate::common::{AgentError, Result};

use super::Project;
use super::items::{TaskWriteOp, clean_text, live_slot};
use super::selector::{Sel, parse_item};
use super::session::model::{HandoverTaskRecord, TurnKey, TurnRecord};

impl Project {
    /// Task IDs in `ops` refer to the numbering at the start of the call, so deleting several
    /// tasks in one call never shifts the ones that follow (the same rule as for notes). Tasks
    /// appended in the call continue that numbering. When the call is done the remaining tasks
    /// are renumbered without gaps.
    pub(crate) fn apply_task_ops(&self, turn: &mut TurnRecord, ops: &[TaskWriteOp]) -> Result<()> {
        let key = turn.key;
        let mut slots: Vec<Option<HandoverTaskRecord>> =
            turn.tasks.iter().cloned().map(Some).collect();
        for op in ops {
            match op {
                TaskWriteOp::Append {
                    task_content,
                    user_explicit,
                } => {
                    slots.push(Some(HandoverTaskRecord {
                        content: clean_text(task_content, "task_content")?,
                        user_explicit: user_explicit.unwrap_or(false),
                        rolled_over_from: None,
                    }));
                }
                TaskWriteOp::Edit {
                    task_id,
                    task_content,
                    user_explicit,
                } => {
                    let i = live_task(&slots, task_id, key)?;
                    let content = clean_text(task_content, "task_content")?;
                    if let Some(task) = slots[i].as_mut() {
                        task.content = content;
                        if let Some(flag) = user_explicit {
                            task.user_explicit = *flag;
                        }
                    }
                }
                TaskWriteOp::Delete { task_id } => {
                    let i = live_task(&slots, task_id, key)?;
                    slots[i] = None;
                }
                TaskWriteOp::RollOver { task_id } => {
                    let (id, source) = self.rollover_source(key, task_id)?;
                    if !slots
                        .iter()
                        .flatten()
                        .any(|t| t.rolled_over_from.as_deref() == Some(id.as_str()))
                    {
                        slots.push(Some(HandoverTaskRecord {
                            content: source.content,
                            user_explicit: source.user_explicit,
                            rolled_over_from: Some(id),
                        }));
                    }
                }
            }
        }
        turn.tasks = slots.into_iter().flatten().collect();
        Ok(())
    }

    /// The task being rolled over and its normalized ID. It must belong to an earlier turn.
    fn rollover_source(&self, active: TurnKey, raw: &str) -> Result<(String, HandoverTaskRecord)> {
        let Some(Sel::Task(key, idx)) = parse_item(raw.trim()) else {
            return Err(AgentError::invalid(
                "task_id",
                format!("`{raw}` is not a task ID like 01.02.task.01"),
            ));
        };
        if key >= active {
            return Err(AgentError::invalid(
                "task_id",
                "only tasks of earlier turns can be rolled over",
            ));
        }
        let task = self
            .turn(key)
            .and_then(|t| t.tasks.get(idx))
            .cloned()
            .ok_or_else(|| AgentError::NotFound(format!("task {}", raw.trim())))?;
        Ok((format!("{key}.task.{:02}", idx + 1), task))
    }
}

/// Like [`live_slot`], and an ID may also name the task it was rolled over from.
fn live_task(slots: &[Option<HandoverTaskRecord>], raw: &str, key: TurnKey) -> Result<usize> {
    let id = raw.trim();
    let by_origin = slots.iter().position(|s| {
        s.as_ref()
            .is_some_and(|t| t.rolled_over_from.as_deref() == Some(id))
    });
    match by_origin {
        Some(i) => Ok(i),
        None => live_slot(slots, raw, key, "task"),
    }
}
