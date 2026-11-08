use stateful_agent_common::{PageParams, Range, ServerConfig};
use stateful_agent_langtool::tools::FindReferencesParams;
use stateful_agent_testkit::TempDir;

use crate::support::Tools;

#[test]
fn find_references_finds_definitions_and_uses_but_not_comments() {
    let tmp = TempDir::new("code-references");
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
    let state = Tools::new(ServerConfig::default());
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
    let tmp = TempDir::new("code-references-qualified");
    tmp.write(
        "src/a.rs",
        "impl Store {\n    fn put(&self) {}\n    fn bulk(&self) { self.put(); }\n}\nimpl Queue {\n    fn put(&self) {}\n    fn drain(&self) { self.put(); }\n}\nfn main() { Store::put(); }\n",
    );
    let state = Tools::new(ServerConfig::default());
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

#[test]
fn find_references_takes_several_symbols_in_one_scan() {
    let tmp = TempDir::new("code-references-batch");
    tmp.write(
        "src/a.rs",
        "fn helper() {}\nfn other() { helper(); }\nfn main() { other(); helper(); }\n",
    );
    tmp.write("src/b.rs", "fn caller() { helper(); }\n");
    let state = Tools::new(ServerConfig::default());
    let refs = |symbols: &[&str]| {
        state.langtool_find_references(FindReferencesParams {
            symbol: symbols
                .iter()
                .map(|symbol| symbol.to_string())
                .collect::<Vec<_>>()
                .into(),
            implicit_path: Some(tmp.path_str()),
            ..Default::default()
        })
    };

    // Each file's rows are grouped under the symbol they belong to.
    let out = refs(&["helper", "other"]).unwrap();
    assert!(
        out.contains("# helper\n1:def:fn helper() {}\n2:call:fn other() { helper(); }\n3:call:"),
        "{out}"
    );
    assert!(out.contains("# other\n2:def:fn other()"), "{out}");
    assert_eq!(out.matches("# helper").count(), 2, "{out}");
    assert_eq!(out.matches("# other").count(), 1, "{out}");

    // One symbol keeps the plain output; blanks and repeats are ignored.
    let one = refs(&["helper"]).unwrap();
    assert!(!one.contains('#'), "{one}");
    assert_eq!(refs(&["helper", " helper ", ""]).unwrap(), one);

    // Names without references are reported, with close names when there are some.
    let out = refs(&["other", "zzz"]).unwrap();
    assert!(
        out.contains("# other") && out.contains("no references to `zzz`"),
        "{out}"
    );
    let out = refs(&["zzz", "yyy"]).unwrap();
    assert!(
        out.starts_with("no references to any of `zzz`, `yyy`"),
        "{out}"
    );
    let out = refs(&["helper", "othr"]).unwrap();
    assert!(
        out.contains("no references to `othr`") && out.contains("similar names for `othr`: other"),
        "{out}"
    );

    // The parameter is a string or a list of strings.
    let one: FindReferencesParams = serde_json::from_str(r#"{"symbol": "helper"}"#).unwrap();
    let many: FindReferencesParams =
        serde_json::from_str(r#"{"symbol": ["helper", "other"]}"#).unwrap();
    assert_eq!(one.symbol.cleaned(), ["helper"]);
    assert_eq!(many.symbol.cleaned(), ["helper", "other"]);
}

#[test]
fn find_references_from_symbol_lists_what_a_definition_uses() {
    let tmp = TempDir::new("code-references-from");
    tmp.write(
        "src/store.rs",
        "pub struct Store;\nimpl Store {\n    pub fn new() -> Store { Store }\n}\npub const LIMIT: usize = 3;\n",
    );
    tmp.write(
        "src/a.rs",
        "use crate::store::{LIMIT as MAX, Store};\nfn helper(n: usize) -> usize { n }\nfn go() -> usize {\n    let s = Store::new();\n    let total = helper(MAX);\n    missing_call(s);\n    helper(total)\n}\n",
    );
    let state = Tools::new(ServerConfig::default());
    let from = |names: &[&str]| {
        state.langtool_find_references(FindReferencesParams {
            from_symbol: Some(
                names
                    .iter()
                    .map(|name| name.to_string())
                    .collect::<Vec<_>>()
                    .into(),
            ),
            implicit_path: Some(tmp.path_str()),
            ..Default::default()
        })
    };

    let out = from(&["go"]).unwrap();
    assert!(out.contains("src/a.rs\n3..8\tfn go() -> usize\n"), "{out}");
    // Calls and uses are matched to the project's definitions by name, through import aliases.
    assert!(
        out.contains("  call\thelper\t@5,7\tsrc/a.rs:2 fn helper\n"),
        "{out}"
    );
    assert!(
        out.contains("  use\tLIMIT (as MAX)\t@5\tsrc/store.rs:5 const LIMIT\n"),
        "{out}"
    );
    assert!(
        out.contains("  call\tnew\t@4\tsrc/store.rs:3 fn Store::new\n"),
        "{out}"
    );
    assert!(
        out.contains("  use\tStore\t@4\t") && out.contains("src/store.rs:1 struct Store"),
        "{out}"
    );
    // A call that nothing defines is placed by the imports of its file, here the prelude or the
    // builtins; locals are not listed at all.
    assert!(out.contains("  external\t1 std\n"), "{out}");
    assert!(
        !out.contains("\ttotal\t") && !out.contains("\ts\t"),
        "{out}"
    );

    // A struct uses the types of its fields, and several names share one scan.
    let out = from(&["Store", "helper"]).unwrap();
    assert!(out.contains("fn helper(n: usize) -> usize"), "{out}");

    let out = from(&["zzz"]).unwrap();
    assert!(out.starts_with("no definition `zzz`"), "{out}");
    let out = from(&["helpr"]).unwrap();
    assert!(
        out.starts_with("no definition") && out.contains("similar symbol names: helper"),
        "{out}"
    );
    let out = from(&["go", "zzz"]).unwrap();
    assert!(
        out.contains("fn go()") && out.contains("no definition `zzz`"),
        "{out}"
    );

    let err = state
        .langtool_find_references(FindReferencesParams {
            symbol: "go".into(),
            from_symbol: Some("go".into()),
            implicit_path: Some(tmp.path_str()),
            ..Default::default()
        })
        .unwrap_err();
    assert!(err.to_string().contains("not both"), "{err}");
    let only: FindReferencesParams = serde_json::from_str(r#"{"from_symbol": "go"}"#).unwrap();
    assert_eq!(only.from_symbol.unwrap().cleaned(), ["go"]);
}

#[test]
fn find_references_from_symbol_places_external_names_by_where_they_come_from() {
    let tmp = TempDir::new("code-references-external");
    tmp.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde_json = \"1\"\n",
    );
    tmp.write(
        "src/lib.rs",
        concat!(
            "use serde_json::from_str;\n",
            "pub fn run() {\n",
            "    from_str(\"1\");\n",
            "    serde_json::to_string(&1);\n",
            "    std::fs::read(\"a\");\n",
            "    drop(1);\n",
            "    value.frob();\n",
            "}\n",
        ),
    );
    let out = Tools::new(ServerConfig::default())
        .langtool_find_references(FindReferencesParams {
            from_symbol: Some(vec!["run".to_string()].into()),
            implicit_path: Some(tmp.path_str()),
            ..Default::default()
        })
        .unwrap();
    // `to_string` is placed by the crate it is written with, not taken for a std method.
    assert!(
        out.contains("  external\t2 std, 1 unresolved; deps: serde_json\n"),
        "{out}"
    );
}
