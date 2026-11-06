mod common;
mod workflow {
    mod get_files_diff;
    mod get_turns;
    mod new_turn;
    mod register;
    mod search;
    mod write_files;
    mod write_notes;
    mod write_tasks;
}

use agent_mcp::server::State;
use agent_mcp::workflow::tools::{NewTurnParams, NoteWriteOp, PatchHunk, TaskWriteOp};

pub(crate) fn hunk(old: &str, new: &str, start_line: Option<u64>) -> PatchHunk {
    PatchHunk {
        start_line,
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
