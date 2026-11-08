use stateful_agent_common::ServerConfig;
use stateful_agent_workflow::tools::{
    GetItemsParams, GetTurnsParams, RegisterParams, TaskWriteOp, WriteTasksParams,
};

use crate::support::{Session, TempProject};

#[test]
fn several_task_deletes_in_one_call_use_the_numbering_from_the_start_of_the_call() {
    let tmp = TempProject::new("task-deletes");
    let state = Session::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
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
        .workflow_write_tasks(WriteTasksParams {
            title: None,
            ops: vec![
                append("alpha"),
                append("bravo"),
                append("charlie"),
                append("delta"),
            ],
        })
        .unwrap();

    let err = state
        .workflow_write_tasks(WriteTasksParams {
            title: None,
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
        .workflow_write_tasks(WriteTasksParams {
            title: None,
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
        .workflow_get_tasks(GetItemsParams {
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
fn write_tasks_names_an_untitled_turn_and_keeps_an_existing_title() {
    let tmp = TempProject::new("tasks-title");
    let state = tmp.open(ServerConfig::default());
    let task = |title: Option<&str>| WriteTasksParams {
        title: title.map(Into::into),
        ops: vec![TaskWriteOp::Append {
            task_content: "next step".into(),
            user_explicit: None,
        }],
    };

    // A blank title does not name the turn.
    let out = state.workflow_write_tasks(task(Some("   "))).unwrap();
    assert!(out.get("title").is_none(), "unexpected output: {out}");

    let out = state
        .workflow_write_tasks(task(Some("Plan handover")))
        .unwrap();
    assert_eq!(out["title"], "Plan handover");
    assert!(out.get("note").is_none(), "unexpected output: {out}");

    let out = state.workflow_write_tasks(task(Some("Other"))).unwrap();
    assert_eq!(out["title"], "Plan handover");
    assert!(out.get("note").is_some(), "unexpected output: {out}");

    let out = state.workflow_write_tasks(task(None)).unwrap();
    assert!(out.get("note").is_none(), "unexpected output: {out}");

    let turns = state.workflow_get_turns(GetTurnsParams::default()).unwrap();
    assert_eq!(turns["turns"][0]["title"], "Plan handover");
}
