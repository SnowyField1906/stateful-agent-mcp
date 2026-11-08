//! Tests of the workflow tools, run over a `Session` that needs no server. The hints the server
//! adds to tool output are tested with the server.

mod case_insensitive;
mod external_edits;
mod get_files_diff;
mod get_turns;
mod new_turn;
mod register;
mod search;
mod support;
mod write_files;
mod write_notes;
mod write_tasks;

use stateful_agent_common::LineNumber;
use stateful_agent_workflow::tools::{NewTurnParams, NoteWriteOp, PatchHunk, TaskWriteOp};

use support::Session;

pub(crate) fn hunk(old: &str, new: &str, start_line: Option<u64>) -> PatchHunk {
    PatchHunk {
        start_line: start_line.and_then(LineNumber::new),
        old_text: old.to_string(),
        new_text: new.to_string(),
        symbol: None,
    }
}

pub(crate) fn turn(state: &Session, title: &str, note: &str, task: Option<&str>) {
    state
        .workflow_new_turn(NewTurnParams {
            title: Some(title.into()),
            write_notes: Some(vec![NoteWriteOp::Append {
                note_content: note.into(),
                ref_notes: None,
            }]),
            write_tasks: task.map(|t| {
                vec![TaskWriteOp::Append {
                    task_content: t.into(),
                    user_explicit: Some(true),
                }]
            }),
            write_files: None,
        })
        .unwrap();
}
