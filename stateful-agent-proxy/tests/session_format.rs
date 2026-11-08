//! The proxy reads `.AGENTS/sessions/*.md` on its own, without the workflow crate. These tests
//! write sessions through the workflow tools and check that the proxy reads the same turns.

use stateful_agent_common::ServerConfig;
use stateful_agent_proxy::binary::load_workspace_turns;
use stateful_agent_testkit::TempDir;
use stateful_agent_workflow::Project;
use stateful_agent_workflow::tools::{
    FileOp, NewTurnParams, NoteWriteOp, PatchHunk, RegisterParams, TaskWriteOp, WriteFilesParams,
    new_turn, register, write_files,
};

fn open(dir: &TempDir) -> Project {
    let params = RegisterParams {
        folder_path: dir.path_str(),
        show_instructions: Some(false),
        show_last_turn_info: Some(false),
    };
    register::register(&ServerConfig::default(), params)
        .unwrap()
        .0
}

fn turn(title: &str) -> NewTurnParams {
    NewTurnParams {
        title: Some(title.into()),
        write_notes: None,
        write_tasks: None,
        write_files: None,
    }
}

#[test]
fn the_proxy_reads_the_turns_the_workflow_tools_write() {
    let dir = TempDir::new("proxy-session-format");
    let mut project = open(&dir);

    write_files::write_files(
        &mut project,
        WriteFilesParams {
            ops: vec![FileOp::Create {
                path: "src/a.rs".into(),
                content: "fn a() {}\n".into(),
            }],
            title: Some("Create a".into()),
        },
    )
    .unwrap();
    new_turn::new_turn(
        &mut project,
        NewTurnParams {
            write_notes: Some(vec![NoteWriteOp::Append {
                note_content: "Keep a.rs small".into(),
                ref_notes: None,
            }]),
            ..turn("Decide")
        },
    )
    .unwrap();
    new_turn::new_turn(
        &mut project,
        NewTurnParams {
            write_files: Some(vec![FileOp::Patch {
                path: "src/a.rs".into(),
                hunks: vec![PatchHunk {
                    start_line: None,
                    old_text: "fn a() {}\n".into(),
                    new_text: "fn a() -> u8 { 1 }\n".into(),
                    symbol: None,
                }],
            }]),
            ..turn("Patch a")
        },
    )
    .unwrap();
    new_turn::new_turn(
        &mut project,
        NewTurnParams {
            write_tasks: Some(vec![TaskWriteOp::Append {
                task_content: "Add tests".into(),
                user_explicit: Some(true),
            }]),
            ..turn("Hand over")
        },
    )
    .unwrap();

    let turns = load_workspace_turns(&dir.root);
    let ids: Vec<&str> = turns.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["01.01", "01.02", "01.03", "01.04"]);
    assert!(
        turns.iter().all(|t| !t.is_empty),
        "every turn recorded something: {turns:?}"
    );
    let paths: Vec<Vec<String>> = turns.iter().map(|t| t.updated_paths.clone()).collect();
    assert_eq!(
        paths,
        vec![vec!["src/a.rs"], vec![], vec!["src/a.rs"], vec![]]
    );
}

#[test]
fn a_new_session_reads_as_one_empty_first_turn() {
    let dir = TempDir::new("proxy-empty-session");
    let _project = open(&dir);

    let turns = load_workspace_turns(&dir.root);
    assert_eq!(turns.len(), 1, "{turns:?}");
    assert_eq!(turns[0].id, "01.01");
    assert!(turns[0].is_empty);
    assert!(turns[0].updated_paths.is_empty());
}
