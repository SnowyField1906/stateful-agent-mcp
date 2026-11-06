//! What notes, handover tasks and turn metadata share: the operation types of the `write_*`
//! tools and `workflow_get_*` selectors, and the helpers that validate item text and IDs.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::common::params::string_or_vec;
use crate::common::{AgentError, Result};

use super::selector::{Sel, parse_item};
use super::session::model::TurnKey;

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
    RollOver { task_id: String },
}

/// Parameters for `workflow_get_notes` and `workflow_get_tasks`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetItemsParams {
    /// One or more selectors: session (`01`), turn (`01.02`), range (`01.01..01.03`, `01..02`,
    /// `01.02..` to the active turn), `HEAD`, or an item ID (`01.02.note.01`, `01.02.task.01`).
    #[serde(deserialize_with = "string_or_vec")]
    pub implicit_ids: Vec<String>,
}

/// Single-line title; empty clears it.
pub(crate) fn clean_title(raw: &str) -> Option<String> {
    let title = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

/// Item text without surrounding blank lines; must contain something other than whitespace.
pub(crate) fn clean_text(raw: &str, arg: &str) -> Result<String> {
    let text = raw.trim_end().trim_start_matches(['\n', '\r']);
    if text.trim().is_empty() {
        return Err(AgentError::invalid(arg, "must not be empty"));
    }
    Ok(text.to_string())
}

/// Zero-based index of an item of the active turn, from `SS.TT.{kind}.NN` or a bare number.
pub(crate) fn item_index(raw: &str, key: TurnKey, kind: &str, len: usize) -> Result<usize> {
    let id = raw.trim();
    let prefix = format!("{key}.{kind}.");
    let number = match id.strip_prefix(&prefix) {
        Some(rest) => rest,
        None if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => id,
        None => {
            let other = match parse_item(id) {
                Some(Sel::Note(k, _)) if kind == "note" => Some(k),
                Some(Sel::Task(k, _)) if kind == "task" => Some(k),
                _ => None,
            };
            return Err(match other {
                Some(k) => AgentError::ClosedTurn(k.to_string()),
                None => AgentError::invalid(
                    format!("{kind}_id"),
                    format!("`{id}` is not an ID of the active turn ({key}.{kind}.NN or NN)"),
                ),
            });
        }
    };
    let n: usize = number.parse().map_err(|_| {
        AgentError::invalid(format!("{kind}_id"), format!("`{id}` is not a number"))
    })?;
    if n == 0 || n > len {
        return Err(AgentError::NotFound(format!(
            "{kind} {id} (turn {key} has {len})"
        )));
    }
    Ok(n - 1)
}

/// Like [`item_index`], for the working copy of an item list in which earlier ops of the same
/// call may already have deleted slots.
pub(crate) fn live_slot<T>(
    slots: &[Option<T>],
    raw: &str,
    key: TurnKey,
    kind: &str,
) -> Result<usize> {
    let i = item_index(raw, key, kind, slots.len())?;
    if slots[i].is_none() {
        return Err(AgentError::NotFound(format!(
            "{kind} {} was already deleted in this call",
            raw.trim()
        )));
    }
    Ok(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes() {
        let key = TurnKey::new(2, 1);
        assert_eq!(item_index("02.01.note.02", key, "note", 3).unwrap(), 1);
        assert_eq!(item_index("3", key, "note", 3).unwrap(), 2);
        assert!(matches!(
            item_index("01.05.note.01", key, "note", 3),
            Err(AgentError::ClosedTurn(_))
        ));
        assert!(item_index("4", key, "note", 3).is_err());
        assert!(item_index("0", key, "note", 3).is_err());
    }

    #[test]
    fn titles_and_text() {
        assert_eq!(clean_title("  a \n b  ").as_deref(), Some("a b"));
        assert_eq!(clean_title("   "), None);
        assert_eq!(clean_text("\n\n  x  \n\n", "n").unwrap(), "  x");
        assert!(clean_text(" \n ", "n").is_err());
    }

    #[test]
    fn note_and_task_ops_use_inline_op_tag() {
        let note: NoteWriteOp = serde_json::from_value(serde_json::json!({
            "op": "Append", "note_content": "decision"
        }))
        .unwrap();
        assert!(matches!(note, NoteWriteOp::Append { .. }));

        let task: TaskWriteOp = serde_json::from_value(serde_json::json!({
            "op": "Delete", "task_id": "01.01.task.01"
        }))
        .unwrap();
        assert!(matches!(task, TaskWriteOp::Delete { .. }));
    }
}
