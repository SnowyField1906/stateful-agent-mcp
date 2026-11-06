use agent_mcp::ServerConfig;
use agent_mcp::common::{PageParams, Range};
use agent_mcp::workflow::tools::GetTurnsParams;

use crate::common::TempProject;
use crate::turn;

#[test]
fn get_turns_shows_the_newest_page_and_pages_by_entries() {
    let tmp = TempProject::new("turns");
    let state = tmp.open(ServerConfig {
        default_limit: 2,
        ..ServerConfig::default()
    });
    turn(&state, "first", "n1", None);
    turn(&state, "second", "n2", Some("t2"));
    turn(&state, "third", "n3", None);

    let out = state.workflow_get_turns(GetTurnsParams::default()).unwrap();
    assert_eq!(out["total"], 3);
    assert_eq!(out["showing"], "2..3");
    let turns = out["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["turn_id"], "01.02");
    assert_eq!(turns[0]["title"], "second");
    assert_eq!(turns[0]["handover_tasks"], 1);
    assert_eq!(turns[1]["turn_id"], "01.03");
    assert_eq!(turns[1]["active"], true);

    let first_page = state
        .workflow_get_turns(GetTurnsParams {
            implicit_ids: None,
            page: PageParams {
                entries: Some("1..2".parse::<Range>().unwrap()),
            },
        })
        .unwrap();
    assert_eq!(first_page["turns"][0]["turn_id"], "01.01");
    assert_eq!(first_page["showing"], "1..2");

    let one = state
        .workflow_get_turns(GetTurnsParams {
            implicit_ids: Some(vec!["01.03".into()]),
            page: PageParams::default(),
        })
        .unwrap();
    assert_eq!(one["total"], 1);
    assert!(one.get("showing").is_none());

    let err = state
        .workflow_get_turns(GetTurnsParams {
            implicit_ids: Some(vec!["01.01.note.01".into()]),
            page: PageParams::default(),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("not a session, turn or range"),
        "{err}"
    );
}
