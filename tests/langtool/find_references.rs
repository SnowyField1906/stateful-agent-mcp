use agent_mcp::ServerConfig;
use agent_mcp::common::{PageParams, Range};
use agent_mcp::langtool::tools::FindReferencesParams;
use agent_mcp::server::State;

use crate::common::TempProject;

#[test]
fn find_references_finds_definitions_and_uses_but_not_comments() {
    let tmp = TempProject::new("code-references");
    tmp.write(
        "src/a.rs",
        "fn helper() {}\n// helper in a comment\nfn main() {\n    helper();\n}\n",
    );
    tmp.write("src/long.rs", "fn validation_action_ran() {}\n");
    tmp.write(
        "src/scoped.rs",
        "mod left {\n    fn helper() {}\n    fn caller() { helper(); }\n}\nmod right {\n    fn helper() {}\n    fn caller() { helper(); }\n}\n",
    );
    tmp.write("m.py", "LIMIT = 3\n\ndef f():\n    return LIMIT\n");
    tmp.write("notes.txt", "helper LIMIT\n");
    let state = State::new(ServerConfig::default());
    let refs = |symbol: &str, f: &dyn Fn(&mut FindReferencesParams)| {
        let mut p = FindReferencesParams {
            symbol: symbol.into(),
            implicit_path: Some(tmp.path_str()),
            ..Default::default()
        };
        f(&mut p);
        state.langtool_find_references(p)
    };

    let rust = refs("helper", &|_| {}).unwrap();
    assert!(rust.contains("src/a.rs"), "unexpected output: {rust}");
    assert!(
        rust.contains("1:def:fn helper() {}") && rust.contains("4:call:helper();"),
        "unexpected output: {rust}"
    );
    assert!(
        !rust.contains("comment") && !rust.contains("notes.txt"),
        "unexpected output: {rust}"
    );

    let scoped = state
        .langtool_find_references(FindReferencesParams {
            symbol: "helper".into(),
            implicit_path: Some(tmp.path_str()),
            scope: Some("LEFT".into()),
            include_scope: Some(true),
            ..Default::default()
        })
        .unwrap();
    assert!(
        scoped.contains("(in left)")
            && scoped.contains("(in left::caller)")
            && !scoped.contains("(in right"),
        "unexpected scoped output: {scoped}"
    );

    let python = refs("LIMIT", &|_| {}).unwrap();
    assert!(
        python.contains("m.py") && python.contains("1:def:LIMIT = 3"),
        "unexpected output: {python}"
    );
    assert!(
        python.contains("4:use:return LIMIT"),
        "unexpected output: {python}"
    );

    assert!(refs("zzz", &|_| {}).unwrap().contains("no references"));
    let typo = refs("helpr", &|_| {}).unwrap();
    assert!(
        typo.contains("no references") && typo.contains("similar names: helper"),
        "{typo}"
    );
    assert!(
        typo.ends_with('\n') && !typo.contains(r"\n"),
        "bad hint separator: {typo:?}"
    );
    let long_typo = refs("validaton_action_ran", &|_| {}).unwrap();
    assert!(
        long_typo.contains("similar names: validation_action_ran"),
        "{long_typo}"
    );
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
fn find_references_accepts_qualified_symbols() {
    let tmp = TempProject::new("code-references-qualified");
    tmp.write(
        "src/a.rs",
        "impl Store {\n    fn put(&self) {}\n    fn bulk(&self) { self.put(); }\n}\nimpl Queue {\n    fn put(&self) {}\n    fn drain(&self) { self.put(); }\n}\nfn main() { Store::put(); }\n",
    );
    let state = State::new(ServerConfig::default());
    let refs = |symbol: &str| {
        state.langtool_find_references(FindReferencesParams {
            symbol: symbol.into(),
            implicit_path: Some(tmp.path_str()),
            ..Default::default()
        })
    };

    for symbol in ["Store::put", "Store.put", "Store/put"] {
        let out = refs(symbol).unwrap();
        assert!(
            out.contains("2:def:") && out.contains("3:call:") && out.contains("9:call:"),
            "{symbol}: {out}"
        );
        assert!(
            !out.contains("6:def:") && !out.contains("7:call:"),
            "{symbol}: {out}"
        );
    }
    assert!(refs("put").unwrap().contains("6:def:"));
    assert!(
        refs("Nothing::put")
            .unwrap()
            .contains("no references to `Nothing::put`")
    );
}
