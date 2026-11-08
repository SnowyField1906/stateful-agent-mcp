use stateful_agent_common::ServerConfig;
use stateful_agent_filesystem::tools::{GrepOutputMode, GrepParams};
use stateful_agent_testkit::TempDir;

use crate::support::Tools;

#[test]
fn grep_searches_with_context_and_output_modes() {
    let tmp = TempDir::new("grep-test");
    tmp.write("f.txt", "a\nb\nc\nd\ne\nf\ng\n");
    tmp.write("other.rs", "fn run() {\n    log();\n}\n");

    let state = Tools::new(ServerConfig::default());

    // Content mode with context
    let out = state
        .filesystem_grep(GrepParams {
            query: "b|f".into(),
            implicit_path: Some(tmp.root.join("f.txt").to_string_lossy().into_owned()),
            context_lines: Some(1),
            output: Some(GrepOutputMode::Content),
            ..Default::default()
        })
        .unwrap();
    assert!(out.contains("2:b"), "unexpected output: {out}");
    assert!(out.contains("6:f"), "unexpected output: {out}");
    assert!(out.contains("1-a"), "unexpected output: {out}");
    assert!(out.contains("--"), "unexpected output: {out}");

    // Files mode
    let files = state
        .filesystem_grep(GrepParams {
            query: "run".into(),
            implicit_path: Some(tmp.path_str()),
            output: Some(GrepOutputMode::Files),
            ..Default::default()
        })
        .unwrap();
    assert!(files.contains("other.rs"), "unexpected files: {files}");

    // Count mode
    let counts = state
        .filesystem_grep(GrepParams {
            query: "log".into(),
            implicit_path: Some(tmp.path_str()),
            output: Some(GrepOutputMode::Count),
            ..Default::default()
        })
        .unwrap();
    assert!(counts.contains("other.rs:1"), "unexpected count: {counts}");
}
