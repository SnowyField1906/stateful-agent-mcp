use stateful_agent_common::ServerConfig;
use stateful_agent_workflow::tools::{
    GetItemsParams, GetTurnsParams, NewTurnParams, NoteWriteOp, RegisterParams, WriteNotesParams,
};

use crate::support::{Session, TempProject};

#[test]
fn several_note_deletes_in_one_call_use_the_numbering_from_the_start_of_the_call() {
    let tmp = TempProject::new("note-deletes");
    let state = Session::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();

    let append = |text: &str| NoteWriteOp::Append {
        note_content: text.into(),
        ref_notes: None,
    };
    state
        .workflow_write_notes(WriteNotesParams {
            title: None,
            ops: vec![
                append("alpha"),
                append("bravo"),
                append("charlie"),
                append("delta"),
            ],
        })
        .unwrap();

    let out = state
        .workflow_write_notes(WriteNotesParams {
            title: None,
            ops: vec![
                NoteWriteOp::Delete {
                    note_id: "01.01.note.01".into(),
                },
                NoteWriteOp::Delete {
                    note_id: "01.01.note.03".into(),
                },
            ],
        })
        .unwrap();
    assert_eq!(
        out["note_ids"],
        serde_json::json!(["01.01.note.01", "01.01.note.02"])
    );

    let notes = state
        .workflow_get_notes(GetItemsParams {
            implicit_ids: vec!["01.01".into()],
        })
        .unwrap()
        .to_string();
    assert!(
        notes.contains("bravo") && notes.contains("delta"),
        "unexpected notes: {notes}"
    );
    assert!(
        !notes.contains("alpha") && !notes.contains("charlie"),
        "unexpected notes: {notes}"
    );
}

#[test]
fn get_notes_expands_ref_notes_recursively() {
    let tmp = TempProject::new("note-refs");
    let state = tmp.open(ServerConfig::default());

    state
        .workflow_write_notes(WriteNotesParams {
            title: None,
            ops: vec![NoteWriteOp::Append {
                note_content: "root decision".into(),
                ref_notes: None,
            }],
        })
        .unwrap();
    state
        .workflow_new_turn(NewTurnParams {
            title: Some("turn 2".into()),
            write_notes: Some(vec![NoteWriteOp::Append {
                note_content: "middle refinement".into(),
                ref_notes: Some(vec!["01.01.note.01".into()]),
            }]),
            write_tasks: None,
            write_files: None,
        })
        .unwrap();
    state
        .workflow_new_turn(NewTurnParams {
            title: Some("turn 3".into()),
            write_notes: Some(vec![NoteWriteOp::Append {
                note_content: "leaf detail".into(),
                ref_notes: Some(vec!["01.02.note.01".into(), "01.01.note.01".into()]),
            }]),
            write_tasks: None,
            write_files: None,
        })
        .unwrap();

    let out = state
        .workflow_get_notes(GetItemsParams {
            implicit_ids: vec!["01.03.note.01".into()],
        })
        .unwrap();
    let leaf = &out[0]["notes"][0];
    assert_eq!(leaf["id"], "01.03.note.01");
    assert_eq!(leaf["content"], "leaf detail");
    let refs = leaf["ref_notes"].as_array().unwrap();
    assert_eq!(
        refs.len(),
        1,
        "diamond ref should not duplicate root: {leaf}"
    );
    assert_eq!(refs[0]["id"], "01.02.note.01");
    assert_eq!(refs[0]["content"], "middle refinement");
    let nested = refs[0]["ref_notes"].as_array().unwrap();
    assert_eq!(nested.len(), 1);
    assert_eq!(nested[0]["id"], "01.01.note.01");
    assert_eq!(nested[0]["content"], "root decision");
    assert!(nested[0].get("ref_notes").is_none());
}

#[test]
fn write_notes_names_an_untitled_turn_and_keeps_an_existing_title() {
    let tmp = TempProject::new("notes-title");
    let state = tmp.open(ServerConfig::default());
    let note = |title: &str| WriteNotesParams {
        title: Some(title.into()),
        ops: vec![NoteWriteOp::Append {
            note_content: "why".into(),
            ref_notes: None,
        }],
    };

    let out = state
        .workflow_write_notes(note("  Decide \n format "))
        .unwrap();
    assert_eq!(out["title"], "Decide format");
    assert!(out.get("note").is_none(), "unexpected output: {out}");

    let out = state.workflow_write_notes(note("Decide format")).unwrap();
    assert!(out.get("note").is_none(), "unexpected output: {out}");

    let out = state.workflow_write_notes(note("Something else")).unwrap();
    assert_eq!(out["title"], "Decide format");
    assert!(out.get("note").is_some(), "unexpected output: {out}");

    let turns = state.workflow_get_turns(GetTurnsParams::default()).unwrap();
    assert_eq!(turns["total"], 1);
    assert_eq!(turns["turns"][0]["title"], "Decide format");
}
