//! Handover tasks of the active turn.

use serde_json::{Value, json};

use crate::error::{AgentError, Result};
use crate::params::TaskWriteOp;
use crate::session::model::{HandoverTaskRecord, TurnKey, TurnRecord};

use super::items::{clean_text, item_index};
use super::project::Project;
use super::selector::{Sel, parse_item};

impl Project {
    pub fn write_tasks(&mut self, ops: &[TaskWriteOp]) -> Result<Value> {
        let mut turn = self.active_turn()?.clone();
        self.apply_task_ops(&mut turn, ops)?;
        let renumbered = ops.iter().any(|op| matches!(op, TaskWriteOp::Delete { .. }));
        let out = tasks_json(&turn, renumbered);
        self.replace_active(turn)?;
        Ok(out)
    }

    /// Task IDs in `ops` refer to the numbering at the start of the call, so deleting several
    /// tasks in one call never shifts the ones that follow (the same rule as for notes). Tasks
    /// appended in the call continue that numbering. When the call is done the remaining tasks
    /// are renumbered without gaps.
    pub(super) fn apply_task_ops(&self, turn: &mut TurnRecord, ops: &[TaskWriteOp]) -> Result<()> {
        let key = turn.key;
        let mut slots: Vec<Option<HandoverTaskRecord>> = turn.tasks.iter().cloned().map(Some).collect();
        for op in ops {
            match op {
                TaskWriteOp::Append { task_content, user_explicit } => {
                    slots.push(Some(HandoverTaskRecord {
                        content: clean_text(task_content, "task_content")?,
                        user_explicit: user_explicit.unwrap_or(false),
                        rolled_over_from: None,
                    }));
                }
                TaskWriteOp::Edit { task_id, task_content, user_explicit } => {
                    let i = live_slot(&slots, task_id, key)?;
                    let content = clean_text(task_content, "task_content")?;
                    if let Some(task) = slots[i].as_mut() {
                        task.content = content;
                        if let Some(flag) = user_explicit {
                            task.user_explicit = *flag;
                        }
                    }
                }
                TaskWriteOp::Delete { task_id } => {
                    let i = live_slot(&slots, task_id, key)?;
                    slots[i] = None;
                }
                TaskWriteOp::RollOver { task_id, task_content, user_explicit } => {
                    let (id, source) = self.rollover_source(key, task_id)?;
                    let content = match task_content {
                        Some(c) => Some(clean_text(c, "task_content")?),
                        None => None,
                    };
                    // Rolling the same task over twice updates the existing copy.
                    if let Some(existing) = slots
                        .iter_mut()
                        .flatten()
                        .find(|t| t.rolled_over_from.as_deref() == Some(id.as_str()))
                    {
                        if let Some(c) = content {
                            existing.content = c;
                        }
                        if let Some(flag) = user_explicit {
                            existing.user_explicit = *flag;
                        }
                    } else {
                        slots.push(Some(HandoverTaskRecord {
                            content: content.unwrap_or(source.content),
                            user_explicit: user_explicit.unwrap_or(source.user_explicit),
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
            return Err(AgentError::invalid("task_id", format!("`{raw}` is not a task ID like 01.02.task.01")));
        };
        if key >= active {
            return Err(AgentError::invalid("task_id", "only tasks of earlier turns can be rolled over"));
        }
        let task = self
            .turn(key)
            .and_then(|t| t.tasks.get(idx))
            .cloned()
            .ok_or_else(|| AgentError::NotFound(format!("task {}", raw.trim())))?;
        Ok((format!("{key}.task.{:02}", idx + 1), task))
    }
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

/// Index of a task that is still present, from its ID in the numbering at the start of the
/// call, a bare number, or the ID it was rolled over from.
fn live_slot(slots: &[Option<HandoverTaskRecord>], raw: &str, key: TurnKey) -> Result<usize> {
    let id = raw.trim();
    let by_origin = slots
        .iter()
        .position(|s| s.as_ref().is_some_and(|t| t.rolled_over_from.as_deref() == Some(id)));
    if let Some(i) = by_origin {
        return Ok(i);
    }
    let i = item_index(raw, key, "task", slots.len())?;
    if slots[i].is_none() {
        return Err(AgentError::NotFound(format!("task {id} was already deleted in this call")));
    }
    Ok(i)
}
