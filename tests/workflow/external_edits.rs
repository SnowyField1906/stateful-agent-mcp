//! Edits made to tracked files outside the ledger are recorded in a turn of their own, so that
//! every turn starts from the content the previous one left and old states stay rebuildable.

use std::fs;

use serde_json::{Value, json};

use agent_mcp::ServerConfig;
use agent_mcp::langtool::tools::{ReadFileEntry, ReadFileParams};
use agent_mcp::server::State;
use agent_mcp::workflow::tools::{
    FileOp, GetFilesDiffParams, GetTurnsParams, NewTurnParams, RegisterParams, WriteFilesParams,
};

use crate::common::TempProject;
use crate::hunk;

const EXTERNAL: &str = "[External disk edit]";

fn register(tmp: &TempProject, last_turn: bool) -> (State, Value) {
    let state = State::new(ServerConfig::default());
    let out = state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(last_turn),
        })
        .unwrap();
    (state, out)
}

fn write(state: &State, title: Option<&str>, ops: Vec<FileOp>) -> Value {
    state
        .workflow_write_files(WriteFilesParams {
            title: title.map(Into::into),
            ops,
        })
        .unwrap()
}

fn create(path: &str, content: &str) -> FileOp {
    FileOp::Create {
        path: path.into(),
        content: content.into(),
    }
}

fn patch(path: &str, old: &str, new: &str) -> FileOp {
    FileOp::Patch {
        path: path.into(),
        hunks: vec![hunk(old, new, None)],
    }
}

fn recover(id: &str, path: &str) -> FileOp {
    FileOp::Recover {
        id: id.into(),
        to_path: Some(path.into()),
    }
}

fn diff(state: &State, id: &str) -> String {
    state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: id.into(),
            implicit_paths: None,
        })
        .unwrap()
}

fn read_at(state: &State, turn: &str, path: &str) -> String {
    state
        .langtool_read_file(ReadFileParams {
            files: vec![ReadFileEntry {
                file_path: path.into(),
                lines: None,
                numbered: Some(false),
                fold_level: None,
                symbol: None,
            }],
            at_implicit_id: Some(turn.into()),
        })
        .unwrap()
}

fn title_of(state: &State, id: &str) -> Value {
    let out = state
        .workflow_get_turns(GetTurnsParams {
            implicit_ids: Some(vec![id.into()]),
            ..GetTurnsParams::default()
        })
        .unwrap();
    out["turns"][0]["title"].clone()
}

fn text(tmp: &TempProject, rel: &str) -> String {
    fs::read_to_string(tmp.root.join(rel)).unwrap()
}

#[test]
fn an_edit_made_between_sessions_becomes_its_own_turn_and_history_stays_exact() {
    let tmp = TempProject::new("external-between-sessions");
    {
        let (state, _) = register(&tmp, false);
        let out = write(
            &state,
            Some("Create"),
            vec![create("a.txt", "alpha\nbeta\ngamma\n")],
        );
        assert!(out.get("external_edits").is_none());
        state
            .workflow_new_turn(NewTurnParams {
                title: Some("Shout beta".into()),
                write_notes: None,
                write_tasks: None,
                write_files: Some(vec![patch("a.txt", "beta\n", "BETA\n")]),
            })
            .unwrap();
    }
    assert_eq!(text(&tmp, ".AGENTS/.known/a.txt"), "alpha\nBETA\ngamma\n");
    fs::write(tmp.root.join("a.txt"), "alpha\nBETA\ngamma\ndelta\n").unwrap();

    let (state, reg) = register(&tmp, true);
    assert_eq!(reg["external_edits"]["turn_id"], "02.01");
    assert_eq!(reg["external_edits"]["paths"], json!(["a.txt"]));
    assert_eq!(reg["active_turn_id"], "02.02");
    // The handover still comes from the last turn of the agent, not from the recorded edit.
    assert_eq!(reg["last_turn"]["turn_id"], "01.02");
    assert_eq!(title_of(&state, "02.01"), EXTERNAL);

    let human = diff(&state, "02.01");
    assert!(human.contains("+delta"), "{human}");
    assert!(!diff(&state, "01.02").contains("delta"));

    let at = |turn: &str| read_at(&state, turn, "a.txt");
    assert!(at("01.01").contains("beta") && !at("01.01").contains("BETA"));
    assert!(at("01.02").contains("BETA") && !at("01.02").contains("delta"));
    assert!(at("02.01").contains("delta"));

    write(&state, Some("Restore"), vec![recover("01.02", "a.txt")]);
    assert_eq!(text(&tmp, "a.txt"), "alpha\nBETA\ngamma\n");
}

#[test]
fn an_edit_between_turns_is_recorded_before_the_next_batch_and_the_turn_diff_stays_clean() {
    let tmp = TempProject::new("external-between-turns");
    let (state, _) = register(&tmp, false);
    write(
        &state,
        Some("Create"),
        vec![create("a.txt", "alpha\nbeta\n")],
    );
    fs::write(tmp.root.join("a.txt"), "alpha\nbeta\ngamma\n").unwrap();

    // The patch only matches the edited text, so the batch runs on what the human left.
    let out = state
        .workflow_new_turn(NewTurnParams {
            title: Some("Add delta".into()),
            write_notes: None,
            write_tasks: None,
            write_files: Some(vec![patch("a.txt", "gamma\n", "gamma\ndelta\n")]),
        })
        .unwrap();
    assert_eq!(out["external_edits"]["turn_id"], "01.02");
    assert_eq!(out["turn_id"], "01.03");
    assert_eq!(title_of(&state, "01.02"), EXTERNAL);

    let human = diff(&state, "01.02");
    assert!(
        human.contains("+gamma") && !human.contains("delta"),
        "{human}"
    );
    let agent = diff(&state, "01.03");
    assert!(agent.contains("+delta"), "{agent}");

    write(&state, None, vec![recover("01.01", "a.txt")]);
    assert_eq!(text(&tmp, "a.txt"), "alpha\nbeta\n");

    // Nothing changed outside the ledger since, so the next call records nothing.
    let out = write(&state, None, vec![create("b.txt", "x\n")]);
    assert!(out.get("external_edits").is_none());
}

#[test]
fn a_tracked_file_deleted_on_disk_is_recorded_and_can_be_recovered() {
    let tmp = TempProject::new("external-delete");
    let (state, _) = register(&tmp, false);
    write(&state, Some("Create"), vec![create("a.txt", "alpha\n")]);
    fs::remove_file(tmp.root.join("a.txt")).unwrap();

    let out = write(&state, None, vec![create("b.txt", "beta\n")]);
    assert_eq!(out["external_edits"]["turn_id"], "01.02");
    assert_eq!(out["external_edits"]["paths"], json!(["a.txt"]));
    assert_eq!(out["turn_id"], "01.03");
    assert!(tmp.root.join(".AGENTS/.deleted/01.02/a.txt").is_file());
    assert!(!tmp.root.join(".AGENTS/.known/a.txt").exists());
    assert!(diff(&state, "01.02").starts_with("Delete a.txt"));

    write(&state, None, vec![recover("01.01", "a.txt")]);
    assert_eq!(text(&tmp, "a.txt"), "alpha\n");
}

#[test]
fn an_edit_to_a_file_the_active_turn_already_touched_stays_in_that_turn() {
    let tmp = TempProject::new("external-same-turn");
    let (state, _) = register(&tmp, false);
    write(&state, Some("Create"), vec![create("a.txt", "alpha\n")]);
    fs::write(tmp.root.join("a.txt"), "alpha\nbeta\n").unwrap();

    let out = write(&state, None, vec![create("b.txt", "x\n")]);
    assert!(out.get("external_edits").is_none());
    assert_eq!(out["turn_id"], "01.01");
    // The turn's net diff, from before the turn to the disk, includes the edit.
    assert!(diff(&state, "01.01").contains("beta"));
    assert_eq!(text(&tmp, ".AGENTS/.known/a.txt"), "alpha\nbeta\n");
}

#[test]
fn files_of_a_ledger_without_known_copies_are_adopted_as_they_are() {
    let tmp = TempProject::new("external-adopt");
    {
        let (state, _) = register(&tmp, false);
        write(&state, Some("Create"), vec![create("a.txt", "alpha\n")]);
    }
    assert_eq!(text(&tmp, ".AGENTS/.known/a.txt"), "alpha\n");
    fs::remove_dir_all(tmp.root.join(".AGENTS/.known")).unwrap();
    fs::write(tmp.root.join("a.txt"), "edited\n").unwrap();

    let (_state, reg) = register(&tmp, false);
    assert!(reg.get("external_edits").is_none());
    assert_eq!(text(&tmp, ".AGENTS/.known/a.txt"), "edited\n");
}
