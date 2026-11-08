use stateful_agent_common::ServerConfig;
use stateful_agent_workflow::tools::SearchParams;

use crate::support::TempProject;
use crate::turn;

#[test]
fn search_finds_titles_notes_and_tasks_newest_first() {
    let tmp = TempProject::new("search");
    let state = tmp.open(ServerConfig::default());
    turn(
        &state,
        "Alpha work",
        "Use edition 2024\nsecond line",
        Some("wire the tool"),
    );
    turn(
        &state,
        "Beta work",
        "beta lesson: check the Edition first",
        None,
    );

    let out = state
        .workflow_search(SearchParams {
            query: "edition".into(),
            case_insensitive: Some(true),
            ..SearchParams::default()
        })
        .unwrap();
    assert_eq!(out["total"], 2);
    assert_eq!(out["hits"][0]["id"], "01.02.note.01");
    assert_eq!(out["hits"][1]["id"], "01.01.note.01");
    assert_eq!(out["hits"][1]["text"], "Use edition 2024");

    // Case-sensitive by default, like `filesystem_grep`.
    let strict = state
        .workflow_search(SearchParams {
            query: "edition".into(),
            ..SearchParams::default()
        })
        .unwrap();
    assert_eq!(strict["total"], 1);

    let kinds = state
        .workflow_search(SearchParams {
            query: "work".into(),
            kinds: Some(vec!["titles".into(), "task".into()]),
            ..SearchParams::default()
        })
        .unwrap();
    let ids: Vec<_> = kinds["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, ["01.02", "01.01"]);

    let task = state
        .workflow_search(SearchParams {
            query: "wire".into(),
            implicit_ids: Some(vec!["01.01".into()]),
            ..SearchParams::default()
        })
        .unwrap();
    assert_eq!(task["hits"][0]["kind"], "task");
    assert_eq!(task["hits"][0]["user_explicit"], true);

    let literal = state
        .workflow_search(SearchParams {
            query: "a.c".into(),
            literal: Some(true),
            ..SearchParams::default()
        })
        .unwrap();
    assert_eq!(literal["total"], 0);

    assert!(
        state
            .workflow_search(SearchParams {
                query: "(".into(),
                ..SearchParams::default()
            })
            .is_err()
    );
    assert!(
        state
            .workflow_search(SearchParams {
                query: String::new(),
                ..SearchParams::default()
            })
            .is_err()
    );
}
