use serde_json::{Value, json};
use agent_mcp::ServerConfig;
use agent_mcp::server::State;

use crate::common::TempProject;

fn grep(state: &State, query: &str, glob: Option<&str>) -> String {
    let mut args = json!({ "query": query });
    if let Some(glob) = glob {
        args["glob"] = json!(glob);
    }
    state
        .filesystem_grep(serde_json::from_value(args).unwrap())
        .unwrap()
}

fn read(state: &State, file: &str, options: Value) -> String {
    let mut entry = json!({ "file_path": file });
    if let (Some(entry), Some(options)) = (entry.as_object_mut(), options.as_object()) {
        entry.extend(options.clone());
    }
    state
        .langtool_read_file(serde_json::from_value(json!({ "files": [entry] })).unwrap())
        .unwrap()
}

#[test]
fn repeated_identifier_greps_are_steered_to_symbol_tools_once_per_streak() {
    let tmp = TempProject::new("grep-streak");
    tmp.write("src/lib.rs", crate::LIB);
    tmp.write("README.md", "put it away\n");
    let state = tmp.open(ServerConfig::default());
    let hinted = |out: &str| out.contains("hint: for code symbols, langtool_find_references");

    assert!(!hinted(&grep(&state, "put", None)));
    assert!(
        hinted(&grep(&state, "Store", None)),
        "the second symbol search in a row should be hinted"
    );
    assert!(
        !hinted(&grep(&state, "open", None)),
        "the hint is given once per streak"
    );

    // A regex search ends the streak, and so does a symbol-aware tool.
    assert!(!hinted(&grep(&state, r"put\(", None)));
    assert!(!hinted(&grep(&state, "put", None)));
    state
        .langtool_find_symbols(serde_json::from_value(json!({ "query": "put" })).unwrap())
        .unwrap();
    assert!(!hinted(&grep(&state, "Store", None)));
    assert!(
        hinted(&grep(&state, "open", None)),
        "a new streak should be hinted again"
    );

    // Words in files that are not code are not symbol searches.
    for _ in 0..3 {
        assert!(!hinted(&grep(&state, "away", Some("*.md"))));
    }
}

#[test]
fn raw_reads_of_long_code_files_are_steered_to_outlines_and_symbols() {
    let tmp = TempProject::new("long-reads");
    let long: String = (0..200).map(|i| format!("fn f{i}() {{}}\n")).collect();
    tmp.write("src/long.rs", &long);
    tmp.write("src/short.rs", crate::LIB);
    tmp.write("notes.txt", &"line\n".repeat(200));
    let state = tmp.open(ServerConfig::default());
    let hint = "hint: src/long.rs has 200 lines; fold_level=0";

    assert!(!read(&state, "src/short.rs", json!({})).contains("hint:"));
    assert!(
        !read(&state, "notes.txt", json!({})).contains("hint:"),
        "only code files are hinted"
    );
    assert!(
        !read(&state, "src/long.rs", json!({ "lines": "1..30" })).contains("hint:"),
        "a named range is already focused"
    );
    assert!(read(&state, "src/long.rs", json!({})).contains(hint));
    assert!(read(&state, "src/long.rs", json!({})).contains(hint));
    assert!(
        !read(&state, "src/long.rs", json!({})).contains("hint:"),
        "two unheeded hints are enough"
    );

    // Reading an outline shows the agent is on course, so the hints start over.
    assert!(!read(&state, "src/long.rs", json!({ "fold_level": 0 })).contains("hint:"));
    assert!(read(&state, "src/long.rs", json!({})).contains(hint));
}
