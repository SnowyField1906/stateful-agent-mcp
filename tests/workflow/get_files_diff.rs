use agent_mcp::ServerConfig;
use agent_mcp::langtool::tools::{ReadFileEntry, ReadFileParams};
use agent_mcp::server::State;
use agent_mcp::workflow::tools::{
    FileOp, GetFilesDiffParams, GetFilesHistoryParams, NewTurnParams, RegisterParams,
    WriteFilesParams,
};

use crate::common::TempProject;
use crate::hunk;
use crate::turn;

#[test]
fn move_and_patch_in_same_turn_preserves_move_and_reconstructs_history() {
    let tmp = TempProject::new("move-patch");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();
    turn(&state, "Setup", "setup", None);

    state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Create {
                path: "src/old.rs".into(),
                content: "fn calc() -> i32 {\n    1\n}\n".into(),
            }],
        })
        .unwrap();

    state
        .workflow_new_turn(NewTurnParams {
            title: Some("Rename and update calc".into()),
            write_notes: None,
            write_tasks: None,
            write_files: Some(vec![FileOp::Move {
                path: "src/old.rs".into(),
                to_path: "src/new.rs".into(),
            }]),
        })
        .unwrap();

    state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Patch {
                path: "src/new.rs".into(),
                hunks: vec![hunk("    1\n", "    42\n", None)],
            }],
        })
        .unwrap();

    let diff = state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: "01.02".into(),
            implicit_paths: None,
        })
        .unwrap();
    assert!(
        diff.starts_with("Move src/old.rs → src/new.rs\nPatch src/new.rs +1 -1 at 2\n@@ -2,1 +2,1 @@\n-    1\n+    42\n"),
        "unexpected diff: {diff}"
    );

    let read_01 = state
        .langtool_read_file(ReadFileParams {
            files: vec![ReadFileEntry {
                file_path: "src/old.rs".into(),
                lines: None,
                numbered: Some(false),
                fold_level: None,
                symbol: None,
            }],
            at_implicit_id: Some("01.01".into()),
        })
        .unwrap();
    assert!(read_01.contains("    1\n"), "unexpected read_01: {read_01}");

    state
        .workflow_new_turn(NewTurnParams {
            title: Some("Second update to calc".into()),
            write_notes: None,
            write_tasks: None,
            write_files: Some(vec![FileOp::Patch {
                path: "src/new.rs".into(),
                hunks: vec![hunk("    42\n", "    99\n", None)],
            }]),
        })
        .unwrap();

    let hist = state
        .workflow_get_files_history(GetFilesHistoryParams {
            implicit_id: None,
            implicit_paths: Some(vec!["src/new.rs".into()]),
        })
        .unwrap();
    assert_eq!(hist["turns"].as_array().unwrap().len(), 3);

    let inv_diff = state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: "01.03..01.02".into(),
            implicit_paths: None,
        })
        .unwrap();
    assert!(
        inv_diff.starts_with("Move src/new.rs → src/old.rs\nPatch src/old.rs"),
        "unexpected inverse diff: {inv_diff}"
    );

    let full_inv = state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: "01.03..01.01".into(),
            implicit_paths: None,
        })
        .unwrap();
    assert_eq!(full_inv, "Delete src/new.rs\n");
}
