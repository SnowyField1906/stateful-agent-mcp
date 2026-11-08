//! Hints the server adds after edits: the configured checks to run and the handover reminder.

use stateful_agent_common::ServerConfig;
use stateful_agent_langtool::tools::RunActionParams;
use stateful_agent_workflow::tools::{FileOp, NewTurnParams, WriteFilesParams};

use crate::hunk;
use crate::support::TempProject;

#[cfg(unix)]
#[test]
fn configured_checks_are_hinted_after_code_edits_and_rearmed_per_turn() {
    let tmp = TempProject::new("validation-hints");
    let state = tmp.open(ServerConfig::default());
    tmp.write(
        ".AGENTS/config.toml",
        "[rust]\nlint = \"echo lint\"\nbuild = \"echo build\"\ntest = \"echo test\"\n",
    );
    crate::turn(&state, "Setup", "setup", None);

    let docs = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Create {
                path: "README.md".into(),
                content: "documentation\n".into(),
            }],
        })
        .unwrap();
    assert!(
        docs.get("hint").is_none(),
        "ordinary files should stay quiet"
    );

    let first = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Create {
                path: "src/lib.rs".into(),
                content: "pub fn first() {}\n".into(),
            }],
        })
        .unwrap();
    let hint = first["hint"]
        .as_str()
        .expect("configured checks should be hinted");
    assert!(hint.contains("Rust: lint, build, test"), "{hint}");
    assert!(hint.contains("langtool_run_action"), "{hint}");

    let second = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Patch {
                path: "src/lib.rs".into(),
                hunks: vec![hunk("first", "second", None)],
            }],
        })
        .unwrap();
    assert!(
        second.get("hint").is_none(),
        "hint repeated in one edit loop"
    );

    let validation = state
        .langtool_run_action(RunActionParams {
            language: "rust".into(),
            action: "lint".into(),
            args: None,
        })
        .unwrap();
    assert!(
        validation.contains("Remaining configured checks for Rust: build, test"),
        "unexpected validation hint: {validation}"
    );
    let remaining = state
        .langtool_run_action(RunActionParams {
            language: "rust".into(),
            action: "build".into(),
            args: None,
        })
        .unwrap();
    assert!(
        remaining.contains("Remaining configured checks for Rust: test"),
        "unexpected remaining-check hint: {remaining}"
    );
    let third = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Patch {
                path: "src/lib.rs".into(),
                hunks: vec![hunk("second", "third", None)],
            }],
        })
        .unwrap();
    assert!(
        third.get("hint").is_some(),
        "a later edit should re-arm the hint"
    );

    let next_turn = state
        .workflow_new_turn(NewTurnParams {
            title: Some("Continue Rust work".into()),
            write_notes: None,
            write_tasks: None,
            write_files: Some(vec![FileOp::Create {
                path: "src/next.rs".into(),
                content: "pub fn next() {}\n".into(),
            }]),
        })
        .unwrap();
    assert!(
        next_turn.get("hint").is_some(),
        "a new turn should start clean"
    );
}

#[test]
fn many_edit_batches_without_handover_tasks_are_hinted_once_per_turn() {
    let tmp = TempProject::new("checkpoint-hints");
    let state = tmp.open(ServerConfig::default());
    crate::turn(&state, "Long turn", "note", None);
    let edit = |n: usize| {
        state
            .workflow_write_files(WriteFilesParams {
                title: None,
                ops: vec![FileOp::Create {
                    path: format!("notes/{n}.md"),
                    content: format!("{n}\n"),
                }],
            })
            .unwrap()
    };

    for n in 1..=3 {
        assert!(edit(n).get("hint").is_none(), "batch {n} should stay quiet");
    }
    let fourth = edit(4);
    let hint = fourth["hint"]
        .as_str()
        .expect("four edit batches without tasks should be hinted");
    assert!(hint.contains("workflow_write_tasks"), "{hint}");
    assert!(
        edit(5).get("hint").is_none(),
        "the reminder is given once per turn"
    );

    // A turn that records tasks is never reminded; the count starts over in the next turn.
    crate::turn(&state, "Checkpointed", "note", Some("next step"));
    for n in 6..=10 {
        assert!(
            edit(n).get("hint").is_none(),
            "batch {n} has handover tasks recorded"
        );
    }
    crate::turn(&state, "Unplanned again", "note", None);
    for n in 11..=13 {
        assert!(edit(n).get("hint").is_none(), "batch {n} should stay quiet");
    }
    assert!(
        edit(14).get("hint").is_some(),
        "the count should start over in a new turn"
    );
}

#[cfg(unix)]
#[test]
fn failed_check_stays_pending_until_code_is_fixed() {
    let tmp = TempProject::new("failed-validation-hint");
    let state = tmp.open(ServerConfig::default());
    tmp.write(
        ".AGENTS/config.toml",
        "[rust]\nlint = \"echo lint; exit 1\"\n",
    );

    let first = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Create {
                path: "src/lib.rs".into(),
                content: "pub fn first() {}\n".into(),
            }],
        })
        .unwrap();
    assert!(first.get("hint").is_some());

    let report = state
        .langtool_run_action(RunActionParams {
            language: "rust".into(),
            action: "lint".into(),
            args: None,
        })
        .unwrap();
    assert!(report.contains("exit 1"), "unexpected report: {report}");
    assert!(!report.contains("Remaining configured checks"));

    let fixed = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Patch {
                path: "src/lib.rs".into(),
                hunks: vec![hunk("first", "fixed", None)],
            }],
        })
        .unwrap();
    assert!(
        fixed.get("hint").is_some(),
        "a failed check should be suggested again after a code fix"
    );
}

#[cfg(unix)]
#[test]
fn broad_recover_hints_from_actual_changed_source_paths() {
    let tmp = TempProject::new("recover-validation-hint");
    let state = tmp.open(ServerConfig::default());
    tmp.write(".AGENTS/config.toml", "[rust]\nlint = \"echo lint\"\n");
    crate::turn(&state, "Setup", "setup", None);

    state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Create {
                path: "src/lib.rs".into(),
                content: "pub fn value() { 1 }\n".into(),
            }],
        })
        .unwrap();
    let next_turn = state
        .workflow_new_turn(NewTurnParams {
            title: Some("Change the value".into()),
            write_notes: None,
            write_tasks: None,
            write_files: Some(vec![FileOp::Patch {
                path: "src/lib.rs".into(),
                hunks: vec![hunk("1", "2", None)],
            }]),
        })
        .unwrap();
    assert!(next_turn.get("hint").is_some());

    state
        .langtool_run_action(RunActionParams {
            language: "rust".into(),
            action: "lint".into(),
            args: None,
        })
        .unwrap();
    let recovered = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Recover {
                id: "01.01".into(),
                to_path: None,
            }],
        })
        .unwrap();

    assert!(
        recovered.get("hint").is_some(),
        "recovery changed a Rust file"
    );
    assert!(
        recovered.get("changed_paths").is_none(),
        "internal paths leaked into MCP output"
    );
}
