//! `Workspace::work_front`: the files and the open work of the latest sessions.

use stateful_agent_common::{Front, ItemKind, ServerConfig};
use stateful_agent_workflow::tools::{
    FileOp, NewTurnParams, NoteWriteOp, RegisterParams, TaskWriteOp,
};

use crate::support::{Session, TempProject};

/// A turn that creates `files` and records a note and, when given, a handover task.
fn step(state: &Session, title: &str, files: &[&str], note: Option<&str>, task: Option<&str>) {
    state
        .workflow_new_turn(NewTurnParams {
            title: Some(title.into()),
            write_notes: note.map(|text| {
                vec![NoteWriteOp::Append {
                    note_content: text.into(),
                    ref_notes: None,
                }]
            }),
            write_tasks: task.map(|text| {
                vec![TaskWriteOp::Append {
                    task_content: text.into(),
                    user_explicit: None,
                }]
            }),
            write_files: Some(
                files
                    .iter()
                    .map(|path| FileOp::Create {
                        path: (*path).into(),
                        content: "x\n".into(),
                    })
                    .collect(),
            ),
        })
        .unwrap();
}

fn paths(front: &Front) -> Vec<&str> {
    front
        .moved
        .iter()
        .map(|moved| moved.path.as_str())
        .collect()
}

fn ids(front: &Front) -> Vec<&str> {
    front.open.iter().map(|item| item.id.as_str()).collect()
}

#[test]
fn a_project_without_history_has_an_empty_front() {
    let tmp = TempProject::new("front-empty");
    let state = tmp.open(ServerConfig::default());
    assert_eq!(state.work_front(), Front::default());
}

#[test]
fn the_front_lists_files_newest_first_and_the_standing_tasks_and_the_notes() {
    let tmp = TempProject::new("front-files");
    let state = tmp.open(ServerConfig::default());
    step(&state, "First", &["a.rs", "b.rs"], Some("n1"), Some("t1"));
    step(&state, "Second", &["c.rs"], Some("n2"), None);

    let front = state.work_front();
    assert_eq!(paths(&front), ["c.rs", "a.rs", "b.rs"]);
    let moved: Vec<(&str, usize, bool)> = front
        .moved
        .iter()
        .map(|moved| (moved.turn.as_str(), moved.age, moved.outside))
        .collect();
    assert_eq!(
        moved,
        [
            ("01.02", 0, false),
            ("01.01", 1, false),
            ("01.01", 1, false)
        ]
    );
    // The second turn passed no task on, so the task of the first still stands.
    assert_eq!(
        ids(&front),
        ["01.02.note.01", "01.01.task.01", "01.01.note.01"]
    );
    let kinds: Vec<ItemKind> = front.open.iter().map(|item| item.kind).collect();
    assert_eq!(kinds, [ItemKind::Note, ItemKind::Task, ItemKind::Note]);
    assert_eq!(front.open[1].text, "t1");
}

#[test]
fn only_the_last_two_sessions_with_records_are_in_the_front() {
    let tmp = TempProject::new("front-sessions");
    let state = tmp.open(ServerConfig::default());
    step(&state, "Old", &["old.rs"], Some("old note"), None);
    for (title, file) in [("Middle", "middle.rs"), ("Latest", "latest.rs")] {
        state
            .workflow_register(RegisterParams {
                folder_path: tmp.path_str(),
                show_instructions: Some(false),
                show_last_turn_info: Some(false),
            })
            .unwrap();
        step(&state, title, &[file], None, None);
    }

    let front = state.work_front();
    assert_eq!(paths(&front), ["latest.rs", "middle.rs"]);
    assert!(
        front.open.is_empty(),
        "the note of session 1 is in the front"
    );
}
