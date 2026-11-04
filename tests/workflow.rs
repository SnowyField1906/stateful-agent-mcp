use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use coding_agent_mcp::params::{
    CodeCallsParams, CodeReferencesParams, CodeSymbolsParams, CreateNewTurnParams, DiffParams,
    FileOp, FsLsParams, FsReadFileParams, FsReadFilesParams, HistoryParams, IdsParams, NoteWriteOp,
    PageParams, PatchHunk, Range, RegisterProjectParams, RunProjectActionParams, TaskWriteOp,
    WriteFilesParams, WriteNotesParams, WriteTasksParams,
};
use coding_agent_mcp::state::State;
use coding_agent_mcp::{AgentError, ServerConfig};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct TempProject {
    root: PathBuf,
}

impl TempProject {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "stateful-agent-mcp-test-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self { root: dir }
    }

    fn path_str(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn hunk(old: &str, new: &str, start_line: Option<u64>) -> PatchHunk {
    PatchHunk {
        start_line,
        old_text: old.to_string(),
        new_text: new.to_string(),
    }
}

#[test]
fn create_rejects_existing_file_and_empty_file_can_be_patched() {
    let tmp = TempProject::new("strict-create");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterProjectParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();

    state
        .workflow_write_files(WriteFilesParams {
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
fn move_and_patch_in_same_turn_preserves_move_and_reconstructs_history() {
    let tmp = TempProject::new("move-patch");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterProjectParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();

    state
        .workflow_write_files(WriteFilesParams {
            ops: vec![FileOp::Create {
                path: "src/old.rs".into(),
                content: "fn calc() -> i32 {\n    1\n}\n".into(),
            }],
        })
        .unwrap();

    state
        .workflow_new_turn(CreateNewTurnParams {
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
            ops: vec![FileOp::Patch {
                path: "src/new.rs".into(),
                hunks: vec![hunk("    1\n", "    42\n", None)],
            }],
        })
        .unwrap();

    let diff = state
        .workflow_get_files_diff(DiffParams {
            implicit_id: "01.02".into(),
            implicit_paths: None,
        })
        .unwrap();
    assert!(
        diff.starts_with("Move src/old.rs → src/new.rs\nPatch src/new.rs +1 -1 at 2\n@@ -2,1 +2,1 @@\n-    1\n+    42\n"),
        "unexpected diff: {diff}"
    );

    let read_01 = state
        .langtool_read_file(FsReadFilesParams {
            files: vec![FsReadFileParams {
                file_path: "src/old.rs".into(),
                lines: None,
                numbered: Some(false),
                fold_level: None,
            }],
            at_implicit_id: Some("01.01".into()),
        })
        .unwrap();
    assert!(read_01.contains("    1\n"), "unexpected read_01: {read_01}");

    state
        .workflow_new_turn(CreateNewTurnParams {
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
        .workflow_get_files_history(HistoryParams {
            implicit_id: None,
            implicit_paths: Some(vec!["src/new.rs".into()]),
        })
        .unwrap();
    assert_eq!(hist["turns"].as_array().unwrap().len(), 3);

    let inv_diff = state
        .workflow_get_files_diff(DiffParams {
            implicit_id: "01.03..01.02".into(),
            implicit_paths: None,
        })
        .unwrap();
    assert!(
        inv_diff.starts_with("Move src/new.rs → src/old.rs\nPatch src/old.rs"),
        "unexpected inverse diff: {inv_diff}"
    );

    let full_inv = state
        .workflow_get_files_diff(DiffParams {
            implicit_id: "01.03..01.01".into(),
            implicit_paths: None,
        })
        .unwrap();
    assert_eq!(full_inv, "Delete src/new.rs\n");
}

#[test]
fn deleted_files_persist_and_recover_across_turns_and_restarts() {
    let tmp = TempProject::new("deleted-recover");
    {
        let state = State::new(ServerConfig::default());
        state
            .workflow_register(RegisterProjectParams {
                folder_path: tmp.path_str(),
                show_instructions: Some(false),
                show_last_turn_info: Some(false),
            })
            .unwrap();

        state
            .workflow_write_files(WriteFilesParams {
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
            .workflow_new_turn(CreateNewTurnParams {
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
                ops: vec![FileOp::Create {
                    path: "dir/b.txt".into(),
                    content: "beta v2\n".into(),
                }],
            })
            .unwrap();
        assert!(tmp.root.join(".AGENTS/.deleted/01.02/a.txt").is_file());
        assert!(!tmp.root.join(".AGENTS/.deleted/01.02/dir/b.txt").exists());
    }

    let state2 = State::new(ServerConfig::default());
    state2
        .workflow_register(RegisterProjectParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();

    state2
        .workflow_write_files(WriteFilesParams {
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

#[test]
fn register_project_surfaces_earlier_handover_tasks_after_interrupted_turn() {
    let tmp = TempProject::new("handover-fallback");
    {
        let state = State::new(ServerConfig::default());
        state
            .workflow_register(RegisterProjectParams {
                folder_path: tmp.path_str(),
                show_instructions: Some(false),
                show_last_turn_info: Some(false),
            })
            .unwrap();

        state
            .workflow_write_task(WriteTasksParams {
                ops: vec![TaskWriteOp::Append {
                    task_content: "Finish parser refactor".into(),
                    user_explicit: Some(true),
                }],
            })
            .unwrap();

        state
            .workflow_new_turn(CreateNewTurnParams {
                title: Some("Interrupted turn".into()),
                write_notes: Some(vec![NoteWriteOp::Append {
                    note_content: "Investigated parser entry point".into(),
                    ref_notes: None,
                }]),
                write_tasks: None,
                write_files: None,
            })
            .unwrap();
    }

    let state2 = State::new(ServerConfig::default());
    let reg = state2
        .workflow_register(RegisterProjectParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(true),
        })
        .unwrap();

    assert_eq!(reg["last_turn"]["turn_id"], "01.02");
    assert_eq!(reg["last_turn"]["handover_tasks_from_turn"], "01.01");
    let tasks = reg["last_turn"]["handover_tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["id"], "01.01.task.01");
    assert_eq!(tasks[0]["content"], "Finish parser refactor");

    let reg_again = state2
        .workflow_register(RegisterProjectParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();
    assert_eq!(reg_again["active_turn_id"], "02.01");
    let _ = AgentError::ProjectNotRegistered;
}

#[test]
fn create_new_turn_rejects_a_title_without_operations() {
    let tmp = TempProject::new("title-only");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterProjectParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();

    let err = state
        .workflow_new_turn(CreateNewTurnParams {
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
fn several_note_deletes_in_one_call_use_the_numbering_from_the_start_of_the_call() {
    let tmp = TempProject::new("note-deletes");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterProjectParams {
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
        .workflow_get_notes(IdsParams {
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
fn fs_tools_read_absolute_paths_without_a_registered_project() {
    let tmp = TempProject::new("absolute-read");
    fs::create_dir_all(tmp.root.join("data")).unwrap();
    fs::write(tmp.root.join("data/defs.lua"), "one\ntwo\n").unwrap();
    let state = State::new(ServerConfig::default());

    let file = tmp
        .root
        .join("data/defs.lua")
        .to_string_lossy()
        .into_owned();
    let read = state
        .langtool_read_file(FsReadFilesParams {
            files: vec![FsReadFileParams {
                file_path: file,
                lines: None,
                numbered: Some(false),
                fold_level: None,
            }],
            at_implicit_id: None,
        })
        .unwrap();
    assert!(read.contains("one\ntwo\n"), "unexpected read: {read}");

    let dir = tmp.root.join("data").to_string_lossy().into_owned();
    let listing = state
        .filesystem_ls(FsLsParams {
            folder_path: Some(dir),
            ..Default::default()
        })
        .unwrap();
    assert!(
        listing.contains("defs.lua"),
        "unexpected listing: {listing}"
    );

    let err = state.filesystem_ls(FsLsParams::default()).unwrap_err();
    assert!(
        err.to_string().contains("workflow_register"),
        "unexpected error: {err}"
    );
}

#[test]
fn fs_read_files_accepts_files_as_a_json_string_and_hints_otherwise() {
    let ok: FsReadFilesParams =
        serde_json::from_value(serde_json::json!({ "files": "[{\"file_path\": \"a.txt\"}]" }))
            .unwrap();
    assert_eq!(ok.files.len(), 1);

    let err =
        serde_json::from_value::<FsReadFilesParams>(serde_json::json!({ "files": "not json" }))
            .unwrap_err();
    assert!(
        err.to_string().contains("array of objects"),
        "unexpected error: {err}"
    );
}

#[test]
fn entries_accept_a_tail_range_and_numbered_parses() {
    let tmp = TempProject::new("tail-entries");
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        fs::write(tmp.root.join(name), "x\n").unwrap();
    }
    let state = State::new(ServerConfig::default());

    let listing = state
        .filesystem_ls(FsLsParams {
            folder_path: Some(tmp.path_str()),
            page: PageParams {
                entries: Some(Range {
                    start: -2,
                    end: None,
                }),
            },
            ..Default::default()
        })
        .unwrap();
    assert!(
        listing.contains("c.txt") && listing.contains("d.txt") && !listing.contains("b.txt"),
        "unexpected listing: {listing}"
    );

    let parsed: FsReadFilesParams = serde_json::from_value(
        serde_json::json!({ "files": [{ "file_path": "a.txt", "numbered": false }] }),
    )
    .unwrap();
    assert_eq!(parsed.files[0].numbered, Some(false));
}

#[test]
fn several_task_deletes_in_one_call_use_the_numbering_from_the_start_of_the_call() {
    let tmp = TempProject::new("task-deletes");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterProjectParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();

    let append = |text: &str| TaskWriteOp::Append {
        task_content: text.into(),
        user_explicit: None,
    };
    state
        .workflow_write_task(WriteTasksParams {
            ops: vec![
                append("alpha"),
                append("bravo"),
                append("charlie"),
                append("delta"),
            ],
        })
        .unwrap();

    let err = state
        .workflow_write_task(WriteTasksParams {
            ops: vec![
                TaskWriteOp::Delete {
                    task_id: "01.01.task.02".into(),
                },
                TaskWriteOp::Delete {
                    task_id: "01.01.task.02".into(),
                },
            ],
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("already deleted"),
        "unexpected error: {err}"
    );

    let out = state
        .workflow_write_task(WriteTasksParams {
            ops: vec![
                TaskWriteOp::Delete {
                    task_id: "01.01.task.01".into(),
                },
                TaskWriteOp::Delete {
                    task_id: "01.01.task.03".into(),
                },
            ],
        })
        .unwrap();
    assert_eq!(
        out["task_ids"],
        serde_json::json!(["01.01.task.01", "01.01.task.02"])
    );
    assert!(out.get("renumbered").is_some(), "unexpected output: {out}");

    let tasks = state
        .workflow_get_tasks(IdsParams {
            implicit_ids: vec!["01.01".into()],
        })
        .unwrap()
        .to_string();
    assert!(
        tasks.contains("bravo") && tasks.contains("delta"),
        "unexpected tasks: {tasks}"
    );
    assert!(
        !tasks.contains("alpha") && !tasks.contains("charlie"),
        "unexpected tasks: {tasks}"
    );
}

#[test]
fn create_new_turn_can_roll_over_a_task_of_the_turn_it_closes() {
    let tmp = TempProject::new("same-call-rollover");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterProjectParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();

    state
        .workflow_write_task(WriteTasksParams {
            ops: vec![TaskWriteOp::Append {
                task_content: "Finish parser refactor".into(),
                user_explicit: Some(true),
            }],
        })
        .unwrap();

    let out = state
        .workflow_new_turn(CreateNewTurnParams {
            title: Some("Continue parser work".into()),
            write_notes: None,
            write_tasks: Some(vec![TaskWriteOp::RollOver {
                task_id: "01.01.task.01".into(),
                task_content: None,
                user_explicit: None,
            }]),
            write_files: None,
        })
        .unwrap();
    assert_eq!(out["turn_id"], "01.02");
    assert_eq!(out["task_ids"], serde_json::json!(["01.02.task.01"]));

    let tasks = state
        .workflow_get_tasks(IdsParams {
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
fn failed_create_new_turn_changes_nothing_on_disk_or_in_state() {
    let tmp = TempProject::new("failed-new-turn");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterProjectParams {
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
            ops: vec![append()],
        })
        .unwrap();

    // A note of the closed turn cannot be edited from the new turn.
    let err = state
        .workflow_new_turn(CreateNewTurnParams {
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
            ops: vec![append()],
        })
        .unwrap();
    assert_eq!(out["turn_id"], "01.01");
}

#[test]
fn register_project_reports_language_capabilities_from_config() {
    let tmp = TempProject::new("language-capabilities");
    let state = State::new(ServerConfig::default());
    let register = || {
        state
            .workflow_register(RegisterProjectParams {
                folder_path: tmp.path_str(),
                show_instructions: Some(false),
                show_last_turn_info: Some(false),
            })
            .unwrap()
    };
    let config_path = tmp.root.join(".AGENTS").join("config.toml");

    // No config file: nothing is reported.
    let reg = register();
    assert!(reg.get("language_capabilities").is_none());
    assert!(reg.get("language_config_error").is_none());

    // Actions are listed in canonical order with `{args}` kept; `format` and languages without
    // actions are left out.
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::write(
        &config_path,
        "[rust]\nformat = \"rustfmt\"\ntest = \"cargo test {args}\"\nbuild = \"cargo build\"\n\n[python]\nformat = \"ruff format -\"\n",
    )
    .unwrap();
    let reg = register();
    assert_eq!(
        reg["language_capabilities"],
        serde_json::json!({ "rust": ["build", "test {args}"] })
    );

    // A broken config does not fail registration; the problem is reported instead.
    fs::write(&config_path, "[rust]\nlinter = \"x\"\n").unwrap();
    let reg = register();
    assert!(reg.get("language_capabilities").is_none());
    assert!(
        reg["language_config_error"]
            .as_str()
            .unwrap()
            .contains("linter")
    );
}

#[cfg(unix)]
#[test]
fn write_files_formats_matching_files_and_records_the_formatted_text() {
    let tmp = TempProject::new("formatter");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterProjectParams {
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
        .workflow_get_files_diff(DiffParams {
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
            ops: vec![create("b.rs")],
        })
        .unwrap();
    assert_eq!(out["format_warnings"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read_to_string(tmp.root.join("b.rs")).unwrap(), "abc\n");
}

#[cfg(unix)]
#[test]
fn run_project_action_runs_configured_commands() {
    let tmp = TempProject::new("actions");
    let state = State::new(ServerConfig::default());
    state
        .workflow_register(RegisterProjectParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();
    fs::write(
        tmp.root.join(".AGENTS").join("config.toml"),
        "[rust]\nbuild = \"echo building\"\ntest = \"echo testing {args}\"\n",
    )
    .unwrap();
    let run = |language: &str, action: &str, args: Option<&str>| {
        state.langtool_run_action(RunProjectActionParams {
            language: language.into(),
            action: action.into(),
            args: args.map(String::from),
        })
    };

    let out = run("rust", "build", None).unwrap();
    assert!(
        out.contains("building") && out.contains("exit 0"),
        "unexpected output: {out}"
    );
    assert!(
        run("rust", "test", Some("-q"))
            .unwrap()
            .contains("testing -q")
    );

    // Missing or unexpected `args` still run the command and append a hint.
    let no_args = run("rust", "test", None).unwrap();
    assert!(
        no_args.contains("testing") && no_args.contains("--help"),
        "unexpected output: {no_args}"
    );
    let extra_args = run("rust", "build", Some("--release")).unwrap();
    assert!(
        extra_args.contains("building") && extra_args.contains("takes no arguments"),
        "unexpected output: {extra_args}"
    );

    // Anything not configured is an error.
    let err = run("rust", "lint", None).unwrap_err();
    assert!(
        err.to_string().contains("not configured"),
        "unexpected error: {err}"
    );
    let err = run("python", "build", None).unwrap_err();
    assert!(
        err.to_string().contains("not configured"),
        "unexpected error: {err}"
    );
}

#[test]
fn code_symbols_lists_rust_and_python_definitions_with_filters_and_paging() {
    let tmp = TempProject::new("code-symbols");
    fs::create_dir_all(tmp.root.join("src")).unwrap();
    fs::create_dir_all(tmp.root.join("pkg")).unwrap();
    fs::write(
        tmp.root.join("src/a.rs"),
        "struct P {\n    x: i32,\n}\n\nimpl P {\n    fn new() -> Self {\n        P { x: 0 }\n    }\n}\n",
    )
    .unwrap();
    fs::write(
        tmp.root.join("pkg/m.py"),
        "class Point:\n    def shift(self):\n        pass\n",
    )
    .unwrap();
    fs::write(tmp.root.join("notes.txt"), "not code\n").unwrap();
    let state = State::new(ServerConfig::default());
    let symbols = |path: &std::path::Path, f: &dyn Fn(&mut CodeSymbolsParams)| {
        let mut p = CodeSymbolsParams {
            implicit_path: Some(path.to_string_lossy().into_owned()),
            ..Default::default()
        };
        f(&mut p);
        state.langtool_find_symbols(p)
    };

    // A directory walk covers both languages and skips other files.
    let all = symbols(&tmp.root, &|_| {}).unwrap();
    assert!(
        all.contains("src/a.rs") && all.contains("pkg/m.py"),
        "unexpected output: {all}"
    );
    assert!(all.contains("1..3\tstruct P"), "unexpected output: {all}");
    assert!(
        all.contains("6..8\tfn new (in impl P)"),
        "unexpected output: {all}"
    );
    assert!(all.contains("class Point"), "unexpected output: {all}");
    assert!(
        all.contains("def shift (in class Point)"),
        "unexpected output: {all}"
    );
    assert!(!all.contains("notes.txt"), "unexpected output: {all}");

    // Name and per-language kind filters.
    let by_name = symbols(&tmp.root, &|p| p.query = Some("NEW".into())).unwrap();
    assert!(
        by_name.contains("fn new") && !by_name.contains("struct P"),
        "unexpected output: {by_name}"
    );
    let by_kind = symbols(&tmp.root, &|p| {
        p.kinds = Some(BTreeMap::from([(
            "python".to_string(),
            vec!["class".to_string()],
        )]));
    })
    .unwrap();
    assert!(
        by_kind.contains("class Point") && !by_kind.contains("fn new"),
        "unexpected output: {by_kind}"
    );
    assert!(
        symbols(&tmp.root, &|p| p.query = Some("zzz".into()))
            .unwrap()
            .contains("no symbols")
    );

    // A single file works, and an unsupported one is rejected.
    let one = symbols(&tmp.root.join("pkg/m.py"), &|_| {}).unwrap();
    assert!(
        one.contains("class Point") && !one.contains("fn new"),
        "unexpected output: {one}"
    );
    let err = symbols(&tmp.root.join("notes.txt"), &|_| {}).unwrap_err();
    assert!(
        err.to_string().contains("no code intelligence"),
        "unexpected error: {err}"
    );

    // Paging counts symbols, not files: the last one only.
    let tail = symbols(&tmp.root, &|p| {
        p.page = PageParams {
            entries: Some(Range {
                start: -1,
                end: None,
            }),
        }
    })
    .unwrap();
    assert_eq!(
        tail.lines().filter(|l| l.contains('\t')).count(),
        1,
        "unexpected output: {tail}"
    );
}

#[test]
fn code_references_finds_definitions_and_uses_but_not_comments() {
    let tmp = TempProject::new("code-references");
    fs::create_dir_all(tmp.root.join("src")).unwrap();
    fs::write(
        tmp.root.join("src/a.rs"),
        "fn helper() {}\n// helper in a comment\nfn main() {\n    helper();\n}\n",
    )
    .unwrap();
    fs::write(
        tmp.root.join("m.py"),
        "LIMIT = 3\n\ndef f():\n    return LIMIT\n",
    )
    .unwrap();
    fs::write(tmp.root.join("notes.txt"), "helper LIMIT\n").unwrap();
    let state = State::new(ServerConfig::default());
    let refs = |symbol: &str, f: &dyn Fn(&mut CodeReferencesParams)| {
        let mut p = CodeReferencesParams {
            symbol: symbol.into(),
            implicit_path: Some(tmp.root.to_string_lossy().into_owned()),
            ..Default::default()
        };
        f(&mut p);
        state.langtool_find_references(p)
    };

    let rust = refs("helper", &|_| {}).unwrap();
    assert!(rust.contains("src/a.rs"), "unexpected output: {rust}");
    assert!(
        rust.contains("1:def:fn helper() {}") && rust.contains("4:helper();"),
        "unexpected output: {rust}"
    );
    assert!(
        !rust.contains("comment") && !rust.contains("notes.txt"),
        "unexpected output: {rust}"
    );

    let python = refs("LIMIT", &|_| {}).unwrap();
    assert!(
        python.contains("m.py") && python.contains("1:def:LIMIT = 3"),
        "unexpected output: {python}"
    );
    assert!(
        python.contains("4:return LIMIT"),
        "unexpected output: {python}"
    );

    assert!(refs("zzz", &|_| {}).unwrap().contains("no references"));
    let err = refs(" ", &|_| {}).unwrap_err();
    assert!(
        err.to_string().contains("symbol"),
        "unexpected error: {err}"
    );

    // Paging counts references, not files: the last one only.
    let tail = refs("helper", &|p| {
        p.page = PageParams {
            entries: Some(Range {
                start: -1,
                end: None,
            }),
        }
    })
    .unwrap();
    assert_eq!(
        tail.lines().filter(|l| l.contains(':')).count(),
        1,
        "unexpected output: {tail}"
    );
}

#[test]
fn code_calls_shows_callees_and_callers_across_files() {
    let tmp = TempProject::new("code-calls");
    fs::create_dir_all(tmp.root.join("src")).unwrap();
    fs::write(
        tmp.root.join("src/a.rs"),
        "fn a() {\n    b();\n    ext();\n}\n\nfn b() {\n    c();\n}\n",
    )
    .unwrap();
    fs::write(tmp.root.join("src/c.rs"), "fn c() {\n    a();\n}\n").unwrap();
    fs::write(
        tmp.root.join("m.py"),
        "def g():\n    pass\n\ndef f():\n    g()\n",
    )
    .unwrap();
    fs::write(tmp.root.join("notes.txt"), "a b c\n").unwrap();
    let state = State::new(ServerConfig::default());
    let calls = |symbol: &str, f: &dyn Fn(&mut CodeCallsParams)| {
        let mut p = CodeCallsParams {
            symbol: symbol.into(),
            implicit_path: Some(tmp.root.to_string_lossy().into_owned()),
            ..Default::default()
        };
        f(&mut p);
        state.langtool_view_callgraph(p)
    };

    // Callees, with calls that have no definition marked external.
    let out = calls("a", &|p| p.direction = Some("outgoing".into())).unwrap();
    assert_eq!(
        out, "a\tsrc/a.rs:1..4\noutgoing:\n  b\tsrc/a.rs:6..8\n  ext\t(external)\n",
        "unexpected output: {out}"
    );

    // Both directions by default; the caller is in another file.
    let both = calls("a", &|_| {}).unwrap();
    assert!(
        both.contains("outgoing:") && both.contains("incoming:\n  c\tsrc/c.rs:1..3\n"),
        "unexpected output: {both}"
    );

    // Tree depth follows the chain until it leads back.
    let deep = calls("a", &|p| {
        p.direction = Some("outgoing".into());
        p.tree_depth = Some(3);
    })
    .unwrap();
    assert!(
        deep.contains("    c\tsrc/c.rs:1..3\n") && deep.contains("(cycle)"),
        "unexpected output: {deep}"
    );

    // Python, and narrowing by file_path or line.
    let py = calls("g", &|p| p.direction = Some("incoming".into())).unwrap();
    assert!(py.contains("f\tm.py:4..5"), "unexpected output: {py}");
    assert!(
        calls("a", &|p| p.file_path = Some("src/c.rs".into()))
            .unwrap()
            .contains("no function")
    );
    assert!(
        calls("b", &|p| p.line = Some(7))
            .unwrap()
            .contains("b\tsrc/a.rs:6..8")
    );
    assert!(
        calls("b", &|p| p.line = Some(1))
            .unwrap()
            .contains("no function")
    );

    assert!(calls("zzz", &|_| {}).unwrap().contains("no function"));
    let err = calls(" ", &|_| {}).unwrap_err();
    assert!(
        err.to_string().contains("symbol"),
        "unexpected error: {err}"
    );
    let err = calls("a", &|p| p.direction = Some("sideways".into())).unwrap_err();
    assert!(
        err.to_string().contains("direction"),
        "unexpected error: {err}"
    );
}

#[test]
fn langtool_read_file_folds_deep_blocks_with_fold_level_and_pages_by_source_line() {
    let tmp = TempProject::new("code-depth");
    fs::write(
        tmp.root.join("a.rs"),
        "struct P {\n    x: i32,\n}\n\nimpl P {\n    fn new() -> Self {\n        P { x: 0 }\n    }\n}\n",
    )
    .unwrap();
    fs::write(tmp.root.join("notes.txt"), "not code\n").unwrap();
    let state = State::new(ServerConfig::default());
    let depth = |file: &str, f: &dyn Fn(&mut FsReadFileParams)| {
        let mut item = FsReadFileParams {
            file_path: tmp.root.join(file).to_string_lossy().into_owned(),
            lines: None,
            numbered: None,
            fold_level: Some(1),
        };
        f(&mut item);
        state.langtool_read_file(FsReadFilesParams {
            files: vec![item],
            at_implicit_id: None,
        })
    };

    // fold_level 1 keeps the method signature but folds its body.
    let one = depth("a.rs", &|_| {}).unwrap();
    assert!(
        one.contains("of 9] fold_level 1"),
        "unexpected output: {one}"
    );
    assert!(
        one.contains("fn new() -> Self") && !one.contains("P { x: 0 }"),
        "unexpected output: {one}"
    );
    assert!(one.contains("x: i32"), "unexpected output: {one}");

    // fold_level 2 shows the body; fold_level 0 folds the struct fields as well.
    let two = depth("a.rs", &|p| p.fold_level = Some(2)).unwrap();
    assert!(two.contains("P { x: 0 }"), "unexpected output: {two}");
    let zero = depth("a.rs", &|p| p.fold_level = Some(0)).unwrap();
    assert!(
        !zero.contains("x: i32") && zero.contains("struct P"),
        "unexpected output: {zero}"
    );

    // `lines` selects source lines.
    let part = depth("a.rs", &|p| p.lines = Some(Range::new(5, 6))).unwrap();
    assert!(
        part.contains("impl P") && !part.contains("struct P"),
        "unexpected output: {part}"
    );

    let err = depth("notes.txt", &|_| {}).unwrap_err();
    assert!(
        err.to_string().contains("no code intelligence"),
        "unexpected error: {err}"
    );
}
