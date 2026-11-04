//! Parameters and DTOs for project, turn, note and handover-task tools.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{opt_string_or_vec, string_or_vec};

/// A note as returned to the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NoteDto {
    /// `{session}.{turn}.note.{index}`, for example `01.02.note.01`.
    pub id: String,
    pub content: String,
    /// IDs of related notes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ref_notes: Vec<String>,
}

/// A handover task as returned to the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HandoverTaskDto {
    /// `{session}.{turn}.task.{index}`, for example `01.02.task.01`.
    pub id: String,
    pub content: String,
    /// The user asked for this task explicitly.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub user_explicit: bool,
    /// ID of the task this one was rolled over from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rolled_over_from: Option<String>,
}

/// One change to the active turn's notes.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "op")]
pub enum NoteWriteOp {
    /// Add a note at the end.
    Append {
        note_content: String,
        #[serde(default)]
        ref_notes: Option<Vec<String>>,
    },
    /// Replace a note's content. `note_id` is a full ID of the active turn or its index.
    Edit {
        note_id: String,
        note_content: String,
        #[serde(default)]
        ref_notes: Option<Vec<String>>,
    },
    /// Remove a note. IDs within one call refer to the numbering at the start of the call; the
    /// remaining notes are renumbered when the call is done.
    Delete { note_id: String },
}

/// One change to the active turn's handover tasks.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "op")]
pub enum TaskWriteOp {
    /// Add a task at the end.
    Append {
        task_content: String,
        #[serde(default)]
        user_explicit: Option<bool>,
    },
    /// Replace a task's content. `task_id` is a full ID of the active turn, its index, or the
    /// ID it was rolled over from.
    Edit {
        task_id: String,
        task_content: String,
        #[serde(default)]
        user_explicit: Option<bool>,
    },
    /// Remove a task. IDs within one call refer to the numbering at the start of the call; the
    /// remaining tasks are renumbered when the call is done.
    Delete { task_id: String },
    /// Copy an unfinished task from an earlier turn into the active turn.
    /// Content and `user_explicit` default to the original's.
    RollOver {
        task_id: String,
        #[serde(default)]
        task_content: Option<String>,
        #[serde(default)]
        user_explicit: Option<bool>,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegisterProjectParams {
    /// Absolute path to the project root folder.
    pub folder_path: String,
    /// Include the instruction files in the response (default true).
    #[serde(default)]
    pub show_instructions: Option<bool>,
    /// Include the last turn's notes, handover tasks and update summary (default true).
    #[serde(default)]
    pub show_last_turn_info: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateNewTurnParams {
    /// Title of the new turn.
    #[serde(default)]
    pub title: Option<String>,
    /// Note operations to run in the new turn.
    #[serde(default)]
    pub write_notes: Option<Vec<NoteWriteOp>>,
    /// Handover task operations to run in the new turn.
    #[serde(default)]
    pub write_tasks: Option<Vec<TaskWriteOp>>,
    /// File operations to run in the new turn.
    #[serde(default)]
    pub write_files: Option<Vec<super::files::FileOp>>,
}

impl CreateNewTurnParams {
    /// True when at least one of the three operation lists holds an operation.
    pub fn has_operations(&self) -> bool {
        self.write_notes.as_ref().is_some_and(|ops| !ops.is_empty())
            || self.write_tasks.as_ref().is_some_and(|ops| !ops.is_empty())
            || self.write_files.as_ref().is_some_and(|ops| !ops.is_empty())
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteNotesParams {
    pub ops: Vec<NoteWriteOp>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteTasksParams {
    pub ops: Vec<TaskWriteOp>,
}

#[cfg(test)]
mod write_op_tests {
    use super::{NoteWriteOp, TaskWriteOp};
    use serde_json::json;

    #[test]
    fn note_and_task_ops_use_inline_op_tag() {
        let note: NoteWriteOp = serde_json::from_value(json!({
            "op": "Append", "note_content": "decision"
        })).unwrap();
        assert!(matches!(note, NoteWriteOp::Append { .. }));

        let task: TaskWriteOp = serde_json::from_value(json!({
            "op": "Delete", "task_id": "01.01.task.01"
        })).unwrap();
        assert!(matches!(task, TaskWriteOp::Delete { .. }));
    }
}

/// Parameters for `workflow_get_notes` and `workflow_get_tasks`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IdsParams {
    /// One or more selectors: session (`01`), turn (`01.02`), range (`01.01..01.03`, `01..02`,
    /// `01.02..` to the active turn), `HEAD`, or an item ID (`01.02.note.01`, `01.02.task.01`).
    #[serde(deserialize_with = "string_or_vec")]
    pub implicit_ids: Vec<String>,
}

/// Parameters for `workflow_get_files_diff`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffParams {
    /// Turn (`01.02`), session (`01`), range (`01.02..01.05`; a reversed range gives the inverse
    /// diff), `HEAD`, or update ID (`01.02.update.src/a.rs`).
    pub implicit_id: String,
    /// Limit to these files or directory prefixes.
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    pub implicit_paths: Option<Vec<String>>,
}

/// Parameters for `workflow_get_files_history`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct HistoryParams {
    /// Turn, session or range to inspect (default: all turns).
    #[serde(default)]
    pub implicit_id: Option<String>,
    /// Limit to these files or directory prefixes; renames are followed.
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    pub implicit_paths: Option<Vec<String>>,
}
