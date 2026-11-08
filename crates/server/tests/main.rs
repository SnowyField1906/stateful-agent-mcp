//! Tests of the server: the hints and steering it adds to tool output, over a registered
//! project in a [`State`].

mod steering;
mod support;
mod write_files_hints;

use stateful_agent_common::LineNumber;
use stateful_agent_server::State;
use stateful_agent_workflow::tools::{NewTurnParams, NoteWriteOp, PatchHunk, TaskWriteOp};

pub(crate) use stateful_agent_testkit::LIB;

pub(crate) fn hunk(old: &str, new: &str, start_line: Option<u64>) -> PatchHunk {
    PatchHunk {
        start_line: start_line.and_then(LineNumber::new),
        old_text: old.to_string(),
        new_text: new.to_string(),
        symbol: None,
    }
}

pub(crate) fn turn(state: &State, title: &str, note: &str, task: Option<&str>) {
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
