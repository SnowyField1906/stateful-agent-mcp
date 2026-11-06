use std::fs;

use agent_mcp::server::State;
use agent_mcp::workflow::tools::{
    NewTurnParams, NoteWriteOp, RegisterParams, TaskWriteOp, WriteTasksParams,
};
use agent_mcp::{AgentError, ServerConfig};

use crate::common::TempProject;

#[test]
fn workflow_register_falls_back_to_the_latest_notes_of_a_recent_turn() {
    let tmp = TempProject::new("notes-fallback");
    {
        let state = State::new(ServerConfig::default());
        state
            .workflow_register(RegisterParams {
                folder_path: tmp.path_str(),
                show_instructions: Some(false),
                show_last_turn_info: Some(false),
            })
            .unwrap();
        crate::turn(&state, "Design", "Keep the ledger hunk-based", None);
        state
            .workflow_new_turn(NewTurnParams {
                title: Some("Small fix".into()),
                write_notes: None,
                write_tasks: Some(vec![TaskWriteOp::Append {
                    task_content: "Follow up".into(),
                    user_explicit: None,
                }]),
                write_files: None,
            })
            .unwrap();
    }

    let state = State::new(ServerConfig::default());
    let reg = state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(true),
        })
        .unwrap();
    assert_eq!(reg["last_turn"]["turn_id"], "01.02");
    assert_eq!(reg["last_turn"]["notes_from_turn"], "01.01");
    let notes = reg["last_turn"]["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["content"], "Keep the ledger hunk-based");
}

#[test]
fn workflow_register_surfaces_earlier_handover_tasks_after_interrupted_turn() {
    let tmp = TempProject::new("handover-fallback");
    {
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

        state
            .workflow_new_turn(NewTurnParams {
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
        .workflow_register(RegisterParams {
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
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();
    assert_eq!(reg_again["active_turn_id"], "02.01");
    let _ = AgentError::ProjectNotRegistered;
}

#[test]
fn workflow_register_reports_language_capabilities_from_config() {
    let tmp = TempProject::new("language-capabilities");
    let state = State::new(ServerConfig::default());
    let register = || {
        state
            .workflow_register(RegisterParams {
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
