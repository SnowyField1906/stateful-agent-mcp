use std::fs;

use serde_json::json;

use stateful_agent_common::ServerConfig;
use stateful_agent_workflow::tools::{
    FileOp, GetFilesDiffParams, GetTurnsParams, NewTurnParams, NoteWriteOp, RegisterParams,
    WriteFilesParams, WriteNotesParams, WriteTasksParams,
};

use crate::hunk;
use crate::support::{Session, TempProject};

#[test]
fn create_rejects_existing_file_and_empty_file_can_be_patched() {
    let tmp = TempProject::new("strict-create");
    let state = Session::new(ServerConfig::default());
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
            ops: vec![
                FileOp::Create {
                    path: "empty.txt".into(),
                    content: String::new(),
                },
                FileOp::Create {
                    path: "existing.txt".into(),
                    content: "v1\n".into(),
                },
            ],
        })
        .unwrap();

    let err = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Create {
                path: "existing.txt".into(),
                content: "v2\n".into(),
            }],
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("already exists; use Patch"),
        "unexpected error: {err}"
    );

    state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Patch {
                path: "empty.txt".into(),
                hunks: vec![hunk("", "filled\n", None)],
            }],
        })
        .unwrap();
    assert_eq!(
        fs::read_to_string(tmp.root.join("empty.txt")).unwrap(),
        "filled\n"
    );
}

#[test]
fn directory_move_is_recorded_per_file_and_later_batches_still_work() {
    let tmp = TempProject::new("dir-move");
    let state = Session::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();
    crate::turn(&state, "Setup", "setup", None);
    state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![
                FileOp::Create {
                    path: "old/a.txt".into(),
                    content: "a\n".into(),
                },
                FileOp::Create {
                    path: "old/b.txt".into(),
                    content: "b\n".into(),
                },
            ],
        })
        .unwrap();

    state
        .workflow_new_turn(NewTurnParams {
            title: Some("Move the directory".into()),
            write_notes: None,
            write_tasks: None,
            write_files: Some(vec![FileOp::Move {
                path: "old".into(),
                to_path: "new".into(),
            }]),
        })
        .unwrap();
    state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Create {
                path: "new/c.txt".into(),
                content: "c\n".into(),
            }],
        })
        .unwrap();

    assert!(tmp.root.join("new/a.txt").is_file());
    assert!(!tmp.root.join("old").exists());
    let diff = state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: "01.02".into(),
            implicit_paths: None,
        })
        .unwrap();
    for expected in [
        "Move old/a.txt → new/a.txt",
        "Move old/b.txt → new/b.txt",
        "Create new/c.txt",
    ] {
        assert!(diff.contains(expected), "missing `{expected}` in: {diff}");
    }
}

#[test]
fn deleted_files_persist_and_recover_across_turns_and_restarts() {
    let tmp = TempProject::new("deleted-recover");
    {
        let state = Session::new(ServerConfig::default());
        state
            .workflow_register(RegisterParams {
                folder_path: tmp.path_str(),
                show_instructions: Some(false),
                show_last_turn_info: Some(false),
            })
            .unwrap();
        crate::turn(&state, "Setup", "setup", None);

        state
            .workflow_write_files(WriteFilesParams {
                title: None,
                ops: vec![
                    FileOp::Create {
                        path: "a.txt".into(),
                        content: "alpha v1\n".into(),
                    },
                    FileOp::Create {
                        path: "dir/b.txt".into(),
                        content: "beta v1\n".into(),
                    },
                ],
            })
            .unwrap();

        state
            .workflow_new_turn(NewTurnParams {
                title: Some("Delete a.txt".into()),
                write_notes: None,
                write_tasks: None,
                write_files: Some(vec![
                    FileOp::Delete {
                        path: "a.txt".into(),
                    },
                    FileOp::Delete {
                        path: "dir/b.txt".into(),
                    },
                ]),
            })
            .unwrap();
        assert!(tmp.root.join(".AGENTS/.deleted/01.02/a.txt").is_file());
        assert!(tmp.root.join(".AGENTS/.deleted/01.02/dir/b.txt").is_file());

        state
            .workflow_write_files(WriteFilesParams {
                title: None,
                ops: vec![FileOp::Create {
                    path: "dir/b.txt".into(),
                    content: "beta v2\n".into(),
                }],
            })
            .unwrap();
        assert!(tmp.root.join(".AGENTS/.deleted/01.02/a.txt").is_file());
        assert!(!tmp.root.join(".AGENTS/.deleted/01.02/dir/b.txt").exists());
    }

    let state2 = Session::new(ServerConfig::default());
    state2
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();

    state2
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Recover {
                id: "01.01".into(),
                to_path: Some("a.txt".into()),
            }],
        })
        .unwrap();
    assert_eq!(
        fs::read_to_string(tmp.root.join("a.txt")).unwrap(),
        "alpha v1\n"
    );

    state2
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![
                FileOp::Delete {
                    path: "a.txt".into(),
                },
                FileOp::Recover {
                    id: "01.02.update.a.txt".into(),
                    to_path: None,
                },
            ],
        })
        .unwrap();
    assert_eq!(
        fs::read_to_string(tmp.root.join("a.txt")).unwrap(),
        "alpha v1\n"
    );

    state2
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Recover {
                id: "01.01".into(),
                to_path: Some("dir".into()),
            }],
        })
        .unwrap();
    assert_eq!(
        fs::read_to_string(tmp.root.join("dir/b.txt")).unwrap(),
        "beta v1\n"
    );

    state2
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![
                FileOp::Patch {
                    path: "a.txt".into(),
                    hunks: vec![hunk("alpha v1\n", "alpha v3\n", None)],
                },
                FileOp::Recover {
                    id: "01.01".into(),
                    to_path: None,
                },
            ],
        })
        .unwrap();
    assert_eq!(
        fs::read_to_string(tmp.root.join("a.txt")).unwrap(),
        "alpha v1\n"
    );
}

#[cfg(unix)]
#[test]
fn write_files_formats_matching_files_and_records_the_formatted_text() {
    let tmp = TempProject::new("formatter");
    let state = Session::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();
    let config_path = tmp.root.join(".AGENTS").join("config.toml");
    fs::write(&config_path, "[rust]\nformat = \"tr a-z A-Z\"\n").unwrap();

    let create = |path: &str| FileOp::Create {
        path: path.into(),
        content: "abc\n".into(),
    };
    let out = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![create("a.rs"), create("notes.md")],
        })
        .unwrap();
    assert!(out.get("formatted").is_none());
    assert!(out.get("turn_files").is_none());
    assert_eq!(
        out["updates"],
        serde_json::json!(["Create a.rs (1 lines)", "Create notes.md (1 lines)"])
    );
    assert!(out.get("format_warnings").is_none());
    assert_eq!(fs::read_to_string(tmp.root.join("a.rs")).unwrap(), "ABC\n");
    assert_eq!(
        fs::read_to_string(tmp.root.join("notes.md")).unwrap(),
        "abc\n"
    );

    // The turn records the formatted text, not the text that was submitted.
    let diff = state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: "01.01".into(),
            implicit_paths: Some(vec!["a.rs".into()]),
        })
        .unwrap();
    assert!(
        diff.contains("ABC") && !diff.contains("abc"),
        "unexpected diff: {diff}"
    );

    // A failing formatter is reported and never fails the write.
    fs::write(
        &config_path,
        "[rust]\nformat = \"cat >/dev/null; exit 2\"\n",
    )
    .unwrap();
    let out = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![create("b.rs")],
        })
        .unwrap();
    assert_eq!(out["format_warnings"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read_to_string(tmp.root.join("b.rs")).unwrap(), "abc\n");
}

#[test]
fn symbol_ops_edit_whole_definitions_and_the_ledger_records_hunks() {
    let tmp = TempProject::new("symbol-ops");
    let state = Session::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();
    crate::turn(&state, "Setup", "setup", None);
    let source = "struct Store;\n\nimpl Store {\n    /// Puts.\n    fn put(&self) {\n        log();\n    }\n}\n";
    state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Create {
                path: "a.rs".into(),
                content: source.into(),
            }],
        })
        .unwrap();
    crate::turn(&state, "edit", "note", None);

    let symbol_op = |make: fn(String, String, String) -> FileOp, symbol: &str, content: &str| {
        make("a.rs".into(), symbol.into(), content.into())
    };
    let out = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![
                symbol_op(
                    |path, symbol, content| FileOp::ReplaceSymbol {
                        path,
                        symbol,
                        content,
                    },
                    "Store::put",
                    "fn put(&self, n: u8) {\n    log(n);\n}",
                ),
                symbol_op(
                    |path, symbol, content| FileOp::InsertAfterSymbol {
                        path,
                        symbol,
                        content,
                    },
                    "Store::put",
                    "fn get(&self) {}",
                ),
            ],
        })
        .unwrap();
    assert_eq!(
        out["updates"],
        json!([
            "ReplaceSymbol a.rs Store::put +2 -2 at 5",
            "InsertAfterSymbol a.rs Store::put +2 -0 at 8"
        ])
    );
    let edited = "struct Store;\n\nimpl Store {\n    /// Puts.\n    fn put(&self, n: u8) {\n        log(n);\n    }\n\n    fn get(&self) {}\n}\n";
    assert_eq!(fs::read_to_string(tmp.root.join("a.rs")).unwrap(), edited);

    // The ledger keeps ordinary hunks, never the symbolic operations.
    let diff = state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: "01.02".into(),
            implicit_paths: None,
        })
        .unwrap();
    assert!(
        diff.contains("+    fn get(&self) {}"),
        "unexpected diff: {diff}"
    );
    assert!(!diff.contains("Symbol"), "unexpected diff: {diff}");

    // A failing operation names itself and leaves the file untouched.
    let err = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::InsertBeforeSymbol {
                path: "a.rs".into(),
                symbol: "Store::missing".into(),
                content: "fn x() {}".into(),
            }],
        })
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("ops[0]") && message.contains("no definition"),
        "unexpected error: {message}"
    );
    assert_eq!(fs::read_to_string(tmp.root.join("a.rs")).unwrap(), edited);
}

#[test]
fn write_params_accept_a_json_string_or_a_single_object() {
    let notes: WriteNotesParams = serde_json::from_value(json!({
        "ops": "[{\"op\":\"Append\",\"note_content\":\"a\"}]"
    }))
    .unwrap();
    assert_eq!(notes.ops.len(), 1);

    let tasks: WriteTasksParams = serde_json::from_value(json!({
        "ops": {"op": "Append", "task_content": "t"}
    }))
    .unwrap();
    assert_eq!(tasks.ops.len(), 1);

    let files: WriteFilesParams = serde_json::from_value(json!({
        "ops": [{"op": "Delete", "path": "a"}]
    }))
    .unwrap();
    assert_eq!(files.ops.len(), 1);
    assert!(files.title.is_none());

    let titled: WriteFilesParams = serde_json::from_value(json!({
        "title": "Add a",
        "ops": [{"op": "Delete", "path": "a"}]
    }))
    .unwrap();
    assert_eq!(titled.title.as_deref(), Some("Add a"));

    let turn: NewTurnParams = serde_json::from_value(json!({
        "title": "t",
        "write_notes": {"op": "Append", "note_content": "n"},
        "write_files": null
    }))
    .unwrap();
    assert_eq!(turn.write_notes.as_ref().map(Vec::len), Some(1));
    assert!(turn.write_files.is_none());

    let err = serde_json::from_value::<WriteFilesParams>(json!({"ops": 3})).unwrap_err();
    assert!(err.to_string().contains("expected an array"), "{err}");
    let err =
        serde_json::from_value::<WriteNotesParams>(json!({"ops": [{"op": "Nope"}]})).unwrap_err();
    assert!(err.to_string().contains("item 1"), "{err}");
}

#[test]
fn write_files_names_the_turn_only_when_the_batch_succeeds() {
    let tmp = TempProject::new("files-title");
    let state = tmp.open(ServerConfig::default());
    let create = |path: &str, title: &str| WriteFilesParams {
        title: Some(title.into()),
        ops: vec![FileOp::Create {
            path: path.into(),
            content: "x\n".into(),
        }],
    };

    state
        .workflow_write_files(WriteFilesParams {
            title: Some("Add a".into()),
            ops: vec![FileOp::Patch {
                path: "missing.txt".into(),
                hunks: vec![hunk("x\n", "y\n", None)],
            }],
        })
        .unwrap_err();
    let turns = state.workflow_get_turns(GetTurnsParams::default()).unwrap();
    assert!(
        turns["turns"][0]["title"].is_null(),
        "unexpected turns: {turns}"
    );

    let out = state
        .workflow_write_files(create("a.txt", "Add a"))
        .unwrap();
    assert_eq!(out["title"], "Add a");
    assert!(out.get("note").is_none(), "unexpected output: {out}");

    let out = state
        .workflow_write_files(create("b.txt", "Other"))
        .unwrap();
    assert_eq!(out["title"], "Add a");
    assert!(out.get("note").is_some(), "unexpected output: {out}");

    let turns = state.workflow_get_turns(GetTurnsParams::default()).unwrap();
    assert_eq!(turns["total"], 1);
    assert_eq!(turns["turns"][0]["title"], "Add a");

    // The named turn is settled like any titled turn: the next unit of work opens turn 01.02.
    let out = state
        .workflow_new_turn(NewTurnParams {
            title: Some("Next".into()),
            write_notes: Some(vec![NoteWriteOp::Append {
                note_content: "why".into(),
                ref_notes: None,
            }]),
            write_tasks: None,
            write_files: None,
        })
        .unwrap();
    assert_eq!(out["turn_id"], "01.02");
    assert_eq!(out["closed"], "01.01");
}

#[test]
fn symbol_edit_that_leaves_a_parse_error_warns_unless_a_later_op_repairs_it() {
    let tmp = TempProject::new("symbol-syntax-warning");
    tmp.write("a.rs", "fn helper() {}\n\nfn other() {}\n");
    tmp.write("b.rs", "fn helper() {}\n");
    let state = tmp.open(ServerConfig::default());
    let replace = |path: &str, content: &str| FileOp::ReplaceSymbol {
        path: path.into(),
        symbol: "helper".into(),
        content: content.into(),
    };

    // The edit is applied and reported: a warning, not an error.
    let out = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![replace("a.rs", "fn helper() {")],
        })
        .unwrap();
    let warnings = out["syntax_warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "unexpected output: {out}");
    let warning = warnings[0].as_str().unwrap();
    assert!(
        warning.starts_with("ReplaceSymbol a.rs helper") && warning.contains("syntax error"),
        "unexpected warning: {warning}"
    );
    assert_eq!(
        fs::read_to_string(tmp.root.join("a.rs")).unwrap(),
        "fn helper() {\n\nfn other() {}\n"
    );

    // A later operation of the same batch completes the change: nothing to warn about.
    let out = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![
                replace("b.rs", "fn helper() {\n    1;"),
                FileOp::Patch {
                    path: "b.rs".into(),
                    hunks: vec![hunk("    1;\n", "    1;\n}\n", None)],
                },
            ],
        })
        .unwrap();
    assert!(
        out.get("syntax_warnings").is_none(),
        "unexpected output: {out}"
    );
    assert_eq!(
        fs::read_to_string(tmp.root.join("b.rs")).unwrap(),
        "fn helper() {\n    1;\n}\n"
    );
}

#[test]
fn rename_symbol_renames_across_files_and_the_ledger_records_hunks() {
    let tmp = TempProject::new("rename-symbol");
    tmp.write(".gitignore", "ignored/\n");
    tmp.write("src/a.rs", "pub fn helper() {}\n// helper in a comment\n");
    tmp.write(
        "src/b.rs",
        "use crate::a::helper;\n\nfn main() {\n    helper();\n    let s = \"helper\";\n}\n",
    );
    tmp.write("notes.txt", "helper\n");
    tmp.write("ignored/c.rs", "fn helper() {}\n");
    let state = tmp.open(ServerConfig::default());

    let out = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::RenameSymbol {
                symbol: " helper ".into(),
                new_name: "assist".into(),
                path: None,
            }],
        })
        .unwrap();
    let summary = out["updates"][0].as_str().unwrap();
    assert_eq!(
        summary,
        "RenameSymbol helper → assist: 3 identifiers in 2 files"
    );

    let read = |rel: &str| fs::read_to_string(tmp.root.join(rel)).unwrap();
    assert_eq!(
        read("src/a.rs"),
        "pub fn assist() {}\n// helper in a comment\n"
    );
    assert_eq!(
        read("src/b.rs"),
        "use crate::a::assist;\n\nfn main() {\n    assist();\n    let s = \"helper\";\n}\n"
    );
    assert_eq!(read("notes.txt"), "helper\n");
    assert_eq!(read("ignored/c.rs"), "fn helper() {}\n");

    // The ledger holds ordinary hunks of the changed files, never the rename itself.
    let diff = state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: "01.01".into(),
            implicit_paths: None,
        })
        .unwrap();
    assert!(
        diff.contains("src/a.rs") && diff.contains("src/b.rs") && diff.contains("+pub fn assist"),
        "unexpected diff: {diff}"
    );
    assert!(
        !diff.contains("RenameSymbol") && !diff.contains("notes.txt"),
        "unexpected diff: {diff}"
    );
}

#[test]
fn rename_symbol_honours_the_path_and_sees_files_created_earlier_in_the_batch() {
    let tmp = TempProject::new("rename-symbol-scope");
    tmp.write("src/a.rs", "fn helper() {}\n");
    tmp.write("lib/b.rs", "fn helper() {}\n");
    let state = tmp.open(ServerConfig::default());

    let out = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![
                FileOp::Create {
                    path: "src/new.rs".into(),
                    content: "fn helper() {}\n".into(),
                },
                FileOp::RenameSymbol {
                    symbol: "helper".into(),
                    new_name: "assist".into(),
                    path: Some("src".into()),
                },
            ],
        })
        .unwrap();
    assert_eq!(
        out["updates"][1],
        "RenameSymbol helper → assist in src: 2 identifiers in 2 files"
    );
    let read = |rel: &str| fs::read_to_string(tmp.root.join(rel)).unwrap();
    assert_eq!(read("src/a.rs"), "fn assist() {}\n");
    assert_eq!(read("src/new.rs"), "fn assist() {}\n");
    assert_eq!(read("lib/b.rs"), "fn helper() {}\n");
}

#[test]
fn failed_rename_symbol_changes_nothing() {
    let tmp = TempProject::new("rename-symbol-fails");
    tmp.write("src/a.rs", "fn helper() {}\n");
    tmp.write("src/a.py", "x = 1\nprint(x)\n");
    let state = tmp.open(ServerConfig::default());
    let rename = |symbol: &str, new_name: &str, path: Option<&str>| WriteFilesParams {
        title: None,
        ops: vec![
            FileOp::Create {
                path: "extra.txt".into(),
                content: "x\n".into(),
            },
            FileOp::RenameSymbol {
                symbol: symbol.into(),
                new_name: new_name.into(),
                path: path.map(Into::into),
            },
        ],
    };

    for (symbol, new_name, expected) in [
        ("x", "if", "syntax error"),
        ("helper", "1bad", "plain identifier"),
        ("helper", "helper", "already the name"),
        ("missing", "other", "no identifier named `missing`"),
    ] {
        let err = state
            .workflow_write_files(rename(symbol, new_name, None))
            .unwrap_err();
        assert!(
            err.to_string().contains(expected),
            "{symbol} -> {new_name}: {err}"
        );
    }
    state
        .workflow_write_files(rename("helper", "assist", Some("nowhere")))
        .unwrap_err();

    // Every failure rolled back the earlier `Create` of its batch too.
    assert_eq!(
        fs::read_to_string(tmp.root.join("src/a.rs")).unwrap(),
        "fn helper() {}\n"
    );
    assert_eq!(
        fs::read_to_string(tmp.root.join("src/a.py")).unwrap(),
        "x = 1\nprint(x)\n"
    );
    assert!(!tmp.root.join("extra.txt").exists());
}
