//! Edits made to tracked files outside the ledger are settled when the agent next changes the
//! same file, so that every turn starts from the content the previous one left and old states
//! stay rebuildable. What the formatter did is merged into the turn that wrote the file; other
//! edits become a turn of their own; files the agent leaves alone leave no trace.

use std::fs;

use serde_json::{Value, json};

use stateful_agent_common::ServerConfig;
use stateful_agent_langtool::tools::{ReadFileEntry, ReadFileParams};
use stateful_agent_workflow::tools::{
    FileOp, GetFilesDiffParams, GetTurnsParams, NewTurnParams, RegisterParams, WriteFilesParams,
};

use crate::hunk;
use crate::support::{Session, TempProject};

const EXTERNAL: &str = "[External disk edit]";

/// A formatter that reads stdin and writes stdout, so the ledger can ask it about old content.
const FORMATTER: &str = "[rust]\nformat = \"sed 's/:i32/: i32/g'\"\n";

fn register(tmp: &TempProject, last_turn: bool) -> (Session, Value) {
    let state = Session::new(ServerConfig::default());
    let out = state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(last_turn),
        })
        .unwrap();
    (state, out)
}

fn write(state: &Session, title: Option<&str>, ops: Vec<FileOp>) -> Value {
    state
        .workflow_write_files(WriteFilesParams {
            title: title.map(Into::into),
            ops,
        })
        .unwrap()
}

fn new_turn(state: &Session, title: &str, ops: Vec<FileOp>) -> Value {
    state
        .workflow_new_turn(NewTurnParams {
            title: Some(title.into()),
            write_notes: None,
            write_tasks: None,
            write_files: Some(ops),
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

fn diff(state: &Session, id: &str) -> String {
    state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: id.into(),
            implicit_paths: None,
        })
        .unwrap()
}

fn read_at(state: &Session, turn: &str, path: &str) -> String {
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

fn title_of(state: &Session, id: &str) -> Value {
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

fn set_formatter(tmp: &TempProject) {
    fs::write(tmp.root.join(".AGENTS/config.toml"), FORMATTER).unwrap();
}

#[test]
fn an_edit_made_between_sessions_is_recorded_when_the_agent_next_changes_the_file() {
    let tmp = TempProject::new("external-between-sessions");
    {
        let (state, _) = register(&tmp, false);
        let out = write(
            &state,
            Some("Create"),
            vec![create("a.txt", "alpha\nbeta\ngamma\n")],
        );
        assert!(out.get("external_edits").is_none());
        new_turn(
            &state,
            "Shout beta",
            vec![patch("a.txt", "beta\n", "BETA\n")],
        );
    }
    assert_eq!(text(&tmp, ".AGENTS/.known/a.txt"), "alpha\nBETA\ngamma\n");
    fs::write(tmp.root.join("a.txt"), "alpha\nBETA\ngamma\ndelta\n").unwrap();

    // Registering changes nothing: no batch has asked for the file yet.
    let (state, reg) = register(&tmp, true);
    assert!(reg.get("external_edits").is_none(), "{reg}");
    assert_eq!(reg["active_turn_id"], "02.01");
    assert_eq!(reg["last_turn"]["turn_id"], "01.02");
    // History stays exact without a record of the edit.
    let at = |turn: &str| read_at(&state, turn, "a.txt");
    assert!(at("01.01").contains("beta") && !at("01.01").contains("BETA"));
    assert!(at("01.02").contains("BETA") && !at("01.02").contains("delta"));

    let out = write(
        &state,
        Some("Add epsilon"),
        vec![patch("a.txt", "delta\n", "delta\nepsilon\n")],
    );
    assert_eq!(out["external_edits"]["turn_id"], "02.01");
    assert_eq!(out["external_edits"]["paths"], json!(["a.txt"]));
    assert_eq!(out["turn_id"], "02.02");
    assert_eq!(title_of(&state, "02.01"), EXTERNAL);

    let human = diff(&state, "02.01");
    assert!(
        human.contains("+delta") && !human.contains("epsilon"),
        "{human}"
    );
    assert!(!diff(&state, "01.02").contains("delta"));
    assert!(diff(&state, "02.02").contains("+epsilon"));

    let at = |turn: &str| read_at(&state, turn, "a.txt");
    assert!(at("01.02").contains("BETA") && !at("01.02").contains("delta"));
    assert!(at("02.01").contains("delta") && !at("02.01").contains("epsilon"));

    write(&state, None, vec![recover("01.02", "a.txt")]);
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
    let out = new_turn(
        &state,
        "Add delta",
        vec![patch("a.txt", "gamma\n", "gamma\ndelta\n")],
    );
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
fn edits_to_files_a_batch_does_not_change_leave_no_turn() {
    let tmp = TempProject::new("external-untouched");
    {
        let (state, _) = register(&tmp, false);
        write(
            &state,
            Some("Create"),
            vec![create("a.txt", "alpha\n"), create("b.txt", "beta\n")],
        );
        fs::write(tmp.root.join("a.txt"), "alpha\nhuman\n").unwrap();

        // The batch changes b.txt only: the edit to a.txt is none of its business.
        let out = new_turn(&state, "Shout b", vec![patch("b.txt", "beta\n", "BETA\n")]);
        assert!(out.get("external_edits").is_none(), "{out}");
        assert_eq!(out["turn_id"], "01.02");
    }
    // The ledger still holds what it last saw, which keeps every old state rebuildable.
    assert_eq!(text(&tmp, ".AGENTS/.known/a.txt"), "alpha\n");

    let (state, reg) = register(&tmp, false);
    assert!(reg.get("external_edits").is_none(), "{reg}");
    assert_eq!(reg["active_turn_id"], "02.01");

    // The edit is recorded once the agent changes the file.
    let out = write(
        &state,
        Some("Extend a"),
        vec![patch("a.txt", "human\n", "human\nagent\n")],
    );
    assert_eq!(out["external_edits"]["turn_id"], "02.01");
    assert_eq!(out["external_edits"]["paths"], json!(["a.txt"]));
    assert_eq!(out["turn_id"], "02.02");
    let human = diff(&state, "02.01");
    assert!(
        human.contains("+human") && !human.contains("agent"),
        "{human}"
    );
    assert!(diff(&state, "02.02").contains("+agent"));
    assert_eq!(title_of(&state, "01.02"), "Shout b");
}

#[test]
fn a_tracked_file_deleted_on_disk_is_recorded_when_the_agent_recovers_it() {
    let tmp = TempProject::new("external-delete");
    let (state, _) = register(&tmp, false);
    write(&state, Some("Create"), vec![create("a.txt", "alpha\n")]);
    fs::remove_file(tmp.root.join("a.txt")).unwrap();

    // The file is not part of this batch, so its deletion is not recorded.
    let out = new_turn(&state, "Add b", vec![create("b.txt", "beta\n")]);
    assert!(out.get("external_edits").is_none(), "{out}");
    assert_eq!(out["turn_id"], "01.02");

    // Restoring it changes the file, so the deletion is recorded first.
    let out = write(&state, None, vec![recover("01.01", "a.txt")]);
    assert_eq!(out["external_edits"]["turn_id"], "01.03");
    assert_eq!(out["external_edits"]["paths"], json!(["a.txt"]));
    assert_eq!(out["turn_id"], "01.04");
    assert!(tmp.root.join(".AGENTS/.deleted/01.03/a.txt").is_file());
    assert!(diff(&state, "01.03").starts_with("Delete a.txt"));
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

    let (state, reg) = register(&tmp, false);
    assert!(reg.get("external_edits").is_none());
    let out = write(
        &state,
        Some("Again"),
        vec![patch("a.txt", "edited\n", "edited again\n")],
    );
    assert!(out.get("external_edits").is_none(), "{out}");
    assert_eq!(text(&tmp, ".AGENTS/.known/a.txt"), "edited again\n");
}

#[test]
fn touch_without_content_change_does_not_create_false_external_edit() {
    let tmp = TempProject::new("external-touch");
    let (state, _) = register(&tmp, false);
    write(&state, Some("Create"), vec![create("a.txt", "hello\n")]);
    assert!(tmp.root.join(".AGENTS/.known_index").is_file());

    // Rewrite identical content to change file mtime
    fs::write(tmp.root.join("a.txt"), "hello\n").unwrap();
    let out = new_turn(&state, "Shout", vec![patch("a.txt", "hello\n", "HELLO\n")]);
    assert!(
        out.get("external_edits").is_none(),
        "no false external edit on unchanged content: {out}"
    );

    // Now write genuine change
    fs::write(tmp.root.join("a.txt"), "HELLO world\n").unwrap();
    let out = new_turn(&state, "Again", vec![patch("a.txt", "world\n", "WORLD\n")]);
    assert_eq!(out["external_edits"]["turn_id"], "01.03");
    assert_eq!(text(&tmp, ".AGENTS/.known/a.txt"), "HELLO WORLD\n");
}

#[test]
fn a_change_that_is_only_the_formatter_is_merged_into_the_turn_that_wrote_the_file() {
    let tmp = TempProject::new("external-format-merge");
    let (state, _) = register(&tmp, false);
    // Written before a formatter is configured, so the ledger holds unformatted text.
    write(
        &state,
        Some("Create"),
        vec![create("a.rs", "fn a(x:i32) {}\n")],
    );
    set_formatter(&tmp);
    fs::write(tmp.root.join("a.rs"), "fn a(x: i32) {}\n").unwrap();

    let out = new_turn(
        &state,
        "Add b",
        vec![patch("a.rs", "{}\n", "{}\nfn b() {}\n")],
    );
    assert_eq!(out["turn_id"], "01.02");
    // No turn records the formatting; the response only says where it went.
    assert!(out["external_edits"].get("turn_id").is_none(), "{out}");
    assert_eq!(out["external_edits"]["formatting_merged"]["a.rs"], "01.01");
    assert_eq!(title_of(&state, "01.02"), "Add b");

    let first = diff(&state, "01.01");
    assert!(
        first.contains("x: i32") && !first.contains("x:i32"),
        "{first}"
    );
    let second = diff(&state, "01.02");
    assert!(second.contains("+fn b() {}"), "{second}");
    assert_eq!(
        text(&tmp, ".AGENTS/.known/a.rs"),
        "fn a(x: i32) {}\nfn b() {}\n"
    );
    let at = read_at(&state, "01.01", "a.rs");
    assert!(at.contains("x: i32") && !at.contains("fn b"), "{at}");
}

#[test]
fn a_formatted_edit_leaves_only_the_edit_in_the_external_turn() {
    let tmp = TempProject::new("external-format-and-edit");
    let (state, _) = register(&tmp, false);
    write(
        &state,
        Some("Create"),
        vec![create("a.rs", "fn a(x:i32) {}\nfn b() {}\n")],
    );
    set_formatter(&tmp);
    // The human edited b and saved with the formatter on, which also reshaped a.
    fs::write(tmp.root.join("a.rs"), "fn a(x: i32) {}\nfn b() { 1 }\n").unwrap();

    let out = new_turn(
        &state,
        "Add c",
        vec![patch("a.rs", "fn b() { 1 }\n", "fn b() { 1 }\nfn c() {}\n")],
    );
    assert_eq!(out["external_edits"]["turn_id"], "01.02");
    assert_eq!(out["external_edits"]["paths"], json!(["a.rs"]));
    assert_eq!(out["external_edits"]["formatting_merged"]["a.rs"], "01.01");
    assert_eq!(out["turn_id"], "01.03");
    assert_eq!(title_of(&state, "01.02"), EXTERNAL);

    let human = diff(&state, "01.02");
    assert!(
        human.contains("+fn b() { 1 }") && !human.contains("x:i32"),
        "{human}"
    );
    let first = diff(&state, "01.01");
    assert!(
        first.contains("x: i32") && !first.contains("x:i32"),
        "{first}"
    );
    assert!(diff(&state, "01.03").contains("+fn c() {}"));
}

#[test]
fn a_hand_edit_in_a_file_the_formatter_would_change_is_recorded_whole() {
    let tmp = TempProject::new("external-format-hand-edit");
    let (state, _) = register(&tmp, false);
    write(
        &state,
        Some("Create"),
        vec![create("a.rs", "fn a(x:i32) {}\n")],
    );
    set_formatter(&tmp);
    // Not formatter output: the human added a line and left the rest as it was.
    fs::write(tmp.root.join("a.rs"), "fn a(x:i32) {}\nfn b() {}\n").unwrap();

    let out = new_turn(
        &state,
        "Add c",
        vec![patch("a.rs", "fn b() {}\n", "fn b() {}\nfn c() {}\n")],
    );
    assert_eq!(out["external_edits"]["turn_id"], "01.02");
    assert!(
        out["external_edits"].get("formatting_merged").is_none(),
        "{out}"
    );
    assert!(diff(&state, "01.01").contains("x:i32"));
    assert!(diff(&state, "01.02").contains("+fn b() {}"));
}
