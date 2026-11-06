use std::fs;

use agent_mcp::server::State;
use agent_mcp::workflow::tools::{
    FileOp, GetFilesDiffParams, GetItemsParams, GetTurnsParams, NewTurnParams, NoteWriteOp,
    RegisterParams, TaskWriteOp, WriteFilesParams, WriteNotesParams, WriteTasksParams,
};
use agent_mcp::{AgentError, ServerConfig};

use crate::common::TempProject;
use crate::hunk;

#[test]
fn workflow_new_turn_rejects_a_title_without_operations() {
    let tmp = TempProject::new("title-only");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();

    let err = state
        .workflow_new_turn(NewTurnParams {
            title: Some("Nothing to record".into()),
            write_notes: Some(vec![]),
            write_tasks: None,
            write_files: None,
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("at least one"),
        "unexpected error: {err}"
    );
}

#[test]
fn workflow_new_turn_can_roll_over_a_task_of_the_turn_it_closes() {
    let tmp = TempProject::new("same-call-rollover");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();

    state
        .workflow_write_tasks(WriteTasksParams {
            title: None,
            ops: vec![TaskWriteOp::Append {
                task_content: "Finish parser refactor".into(),
                user_explicit: Some(true),
            }],
        })
        .unwrap();

    let out = state
        .workflow_new_turn(NewTurnParams {
            title: Some("Continue parser work".into()),
            write_notes: None,
            write_tasks: Some(vec![TaskWriteOp::RollOver {
                task_id: "01.01.task.01".into(),
            }]),
            write_files: None,
        })
        .unwrap();
    assert_eq!(out["turn_id"], "01.02");
    assert_eq!(out["task_ids"], serde_json::json!(["01.02.task.01"]));

    let tasks = state
        .workflow_get_tasks(GetItemsParams {
            implicit_ids: vec!["01.02".into()],
        })
        .unwrap()
        .to_string();
    assert!(
        tasks.contains("Finish parser refactor") && tasks.contains("01.01.task.01"),
        "unexpected tasks: {tasks}"
    );
}

#[test]
fn failed_workflow_new_turn_changes_nothing_on_disk_or_in_state() {
    let tmp = TempProject::new("failed-new-turn");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();

    let append = || NoteWriteOp::Append {
        note_content: "first note".into(),
        ref_notes: None,
    };
    state
        .workflow_write_notes(WriteNotesParams {
            title: None,
            ops: vec![append()],
        })
        .unwrap();

    // A note of the closed turn cannot be edited from the new turn.
    let err = state
        .workflow_new_turn(NewTurnParams {
            title: Some("Edit a closed note".into()),
            write_notes: Some(vec![NoteWriteOp::Edit {
                note_id: "01.01.note.01".into(),
                note_content: "changed".into(),
                ref_notes: None,
            }]),
            write_tasks: None,
            write_files: Some(vec![FileOp::Create {
                path: "created.txt".into(),
                content: "x\n".into(),
            }]),
        })
        .unwrap_err();
    assert!(
        matches!(err, AgentError::ClosedTurn(_)),
        "unexpected error: {err}"
    );
    assert!(!tmp.root.join("created.txt").exists());

    // No new turn was opened.
    let out = state
        .workflow_write_notes(WriteNotesParams {
            title: None,
            ops: vec![append()],
        })
        .unwrap();
    assert_eq!(out["turn_id"], "01.01");
}

#[test]
fn workflow_new_turn_titles_an_untitled_turn_that_only_holds_file_edits() {
    let tmp = TempProject::new("title-edit-only-turn");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();
    let create = |path: &str| FileOp::Create {
        path: path.into(),
        content: "x\n".into(),
    };
    state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![create("a.txt")],
        })
        .unwrap();

    let out = state
        .workflow_new_turn(NewTurnParams {
            title: Some("Add files".into()),
            write_notes: Some(vec![NoteWriteOp::Append {
                note_content: "why".into(),
                ref_notes: None,
            }]),
            write_tasks: None,
            write_files: Some(vec![create("b.txt")]),
        })
        .unwrap();
    assert_eq!(out["turn_id"], "01.01");
    assert!(out.get("closed").is_none());

    let turns = state.workflow_get_turns(GetTurnsParams::default()).unwrap();
    assert_eq!(turns["total"], 1);
    assert_eq!(turns["turns"][0]["title"], "Add files");
    let diff = state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: "01.01".into(),
            implicit_paths: None,
        })
        .unwrap();
    assert!(
        diff.contains("a.txt") && diff.contains("b.txt"),
        "unexpected diff: {diff}"
    );
}

#[test]
fn failed_workflow_new_turn_that_reuses_a_turn_keeps_its_earlier_edits() {
    let tmp = TempProject::new("failed-reuse");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();
    state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Create {
                path: "a.txt".into(),
                content: "one\n".into(),
            }],
        })
        .unwrap();

    state
        .workflow_new_turn(NewTurnParams {
            title: Some("Edit a".into()),
            write_notes: None,
            write_tasks: None,
            write_files: Some(vec![FileOp::Patch {
                path: "a.txt".into(),
                hunks: vec![hunk("missing\n", "three\n", None)],
            }]),
        })
        .unwrap_err();

    assert_eq!(fs::read_to_string(tmp.root.join("a.txt")).unwrap(), "one\n");
    let turns = state.workflow_get_turns(GetTurnsParams::default()).unwrap();
    assert_eq!(turns["total"], 1);
    assert!(turns["turns"][0]["title"].is_null());
    let diff = state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: "01.01".into(),
            implicit_paths: None,
        })
        .unwrap();
    assert!(diff.contains("a.txt"), "unexpected diff: {diff}");
}
