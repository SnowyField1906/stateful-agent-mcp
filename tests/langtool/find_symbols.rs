use std::collections::BTreeMap;

use agent_mcp::ServerConfig;
use agent_mcp::common::{PageParams, Range};
use agent_mcp::langtool::tools::FindSymbolsParams;
use agent_mcp::server::State;

use crate::LIB;
use crate::common::TempProject;

fn kinds(language: &str, list: &[&str]) -> Option<BTreeMap<String, Vec<String>>> {
    Some(BTreeMap::from([(
        language.to_string(),
        list.iter().map(|k| k.to_string()).collect(),
    )]))
}

#[test]
fn symbols_show_whole_signatures_and_validate_kinds() {
    let dir = TempProject::new("symbols");
    dir.write("src/lib.rs", LIB);
    let state = State::new(ServerConfig::default());

    let out = state
        .langtool_find_symbols(FindSymbolsParams {
            implicit_path: Some(dir.path_str()),
            query: Some("put".into()),
            ..FindSymbolsParams::default()
        })
        .unwrap();
    assert!(
        out.contains("pub fn put(&self, key: &str) -> Option<String>"),
        "{out}"
    );

    // Long names are accepted for the common kinds.
    let out = state
        .langtool_find_symbols(FindSymbolsParams {
            implicit_path: Some(dir.path_str()),
            kinds: kinds("rust", &["function"]),
            ..FindSymbolsParams::default()
        })
        .unwrap();
    assert!(
        out.contains("fn setup") && !out.contains("struct Store"),
        "{out}"
    );

    let err = state
        .langtool_find_symbols(FindSymbolsParams {
            implicit_path: Some(dir.path_str()),
            kinds: kinds("rust", &["class"]),
            ..FindSymbolsParams::default()
        })
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("unknown rust kind `class`") && message.contains("fn, struct"),
        "{message}"
    );

    let err = state
        .langtool_find_symbols(FindSymbolsParams {
            implicit_path: Some(dir.path_str()),
            kinds: kinds("go", &["fn"]),
            ..FindSymbolsParams::default()
        })
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("unsupported language `go`"), "{message}");
    let supported = agent_mcp::langtool::engine::syntax::registry::supported_names();
    assert!(
        message.contains(&format!("supported: {supported}")),
        "{message}"
    );
}

#[test]
fn find_symbols_suggests_a_close_name_when_a_query_misses() {
    let dir = TempProject::new("symbol-suggestion");
    dir.write(
        "src/lib.rs",
        "fn configure() {}\nfn validation_action_ran() {}\n",
    );
    let state = State::new(ServerConfig::default());

    let out = state
        .langtool_find_symbols(FindSymbolsParams {
            implicit_path: Some(dir.path_str()),
            query: Some("configuer".into()),
            ..FindSymbolsParams::default()
        })
        .unwrap();
    assert!(out.contains("no symbols"), "{out}");
    assert!(out.contains("similar symbol names: configure"), "{out}");

    let long_typo = state
        .langtool_find_symbols(FindSymbolsParams {
            implicit_path: Some(dir.path_str()),
            query: Some("validation_action_rann".into()),
            ..FindSymbolsParams::default()
        })
        .unwrap();
    assert!(
        long_typo.contains("similar symbol names: validation_action_ran"),
        "{long_typo}"
    );

    let out = state
        .langtool_find_symbols(FindSymbolsParams {
            implicit_path: Some(dir.path_str()),
            query: Some("configuer".into()),
            kinds: kinds("rust", &["struct"]),
            ..FindSymbolsParams::default()
        })
        .unwrap();
    assert!(out.contains("no symbols"), "{out}");
    assert!(!out.contains("similar symbol names"), "{out}");
}

#[test]
fn find_symbols_filters_by_enclosing_scope() {
    let dir = TempProject::new("symbol-scope-filter");
    dir.write(
        "src/lib.rs",
        "struct Store;\nimpl Store { fn put(&self) {} }\nstruct Queue;\nimpl Queue { fn put(&self) {} fn get(&self) {} }\nfn put() {}\n",
    );
    let state = State::new(ServerConfig::default());
    let out = state
        .langtool_find_symbols(FindSymbolsParams {
            implicit_path: Some(dir.path_str()),
            query: Some("put".into()),
            scope: Some("store".into()),
            ..FindSymbolsParams::default()
        })
        .unwrap();

    assert!(out.contains("fn put(&self) (in Store)"), "{out}");
    assert!(!out.contains("Queue") && !out.contains("fn put()"), "{out}");

    let miss = state
        .langtool_find_symbols(FindSymbolsParams {
            implicit_path: Some(dir.path_str()),
            query: Some("gett".into()),
            scope: Some("store".into()),
            ..FindSymbolsParams::default()
        })
        .unwrap();
    assert!(miss.contains("no symbols"), "{miss}");
    assert!(!miss.contains("similar symbol names"), "{miss}");
}

#[test]
fn find_symbols_accepts_qualified_queries_in_every_separator_style() {
    let dir = TempProject::new("symbol-qualified-query");
    dir.write(
        "src/lib.rs",
        "struct Store;\nimpl Store { fn put(&self) {} }\nstruct Queue;\nimpl Queue { fn put(&self) {} }\nfn put() {}\n",
    );
    let state = State::new(ServerConfig::default());
    for query in ["Store::put", "Store.put", "Store/put"] {
        let out = state
            .langtool_find_symbols(FindSymbolsParams {
                implicit_path: Some(dir.path_str()),
                query: Some(query.into()),
                ..FindSymbolsParams::default()
            })
            .unwrap();
        assert!(out.contains("fn put(&self) (in Store)"), "{query}: {out}");
        assert!(
            !out.contains("Queue") && !out.contains("fn put()"),
            "{query}: {out}"
        );
    }
}

#[test]
fn find_symbols_lists_rust_and_python_definitions_with_filters_and_paging() {
    let tmp = TempProject::new("code-symbols");
    tmp.write(
        "src/a.rs",
        "struct P {\n    x: i32,\n}\n\nimpl P {\n    fn new() -> Self {\n        P { x: 0 }\n    }\n}\n",
    );
    tmp.write(
        "pkg/m.py",
        "class Point:\n    def shift(self):\n        pass\n",
    );
    tmp.write("notes.txt", "not code\n");
    let state = State::new(ServerConfig::default());
    let symbols = |path: &std::path::Path, f: &dyn Fn(&mut FindSymbolsParams)| {
        let mut p = FindSymbolsParams {
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
        all.contains("6..8\t  fn new() -> Self"),
        "unexpected output: {all}"
    );
    assert!(all.contains("class Point"), "unexpected output: {all}");
    assert!(
        all.contains("2..3\t  def shift(self)"),
        "unexpected output: {all}"
    );
    assert!(!all.contains("notes.txt"), "unexpected output: {all}");

    // Name and per-language kind filters.
    let by_name = symbols(&tmp.root, &|p| p.query = Some("NEW".into())).unwrap();
    assert!(
        by_name.contains("fn new") && !by_name.contains("struct P"),
        "unexpected output: {by_name}"
    );
    let by_kind = symbols(&tmp.root, &|p| p.kinds = kinds("python", &["class"])).unwrap();
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
    // The page left the parent out, so the row names its scope.
    assert!(
        tail.contains("fn new() -> Self (in P)"),
        "unexpected output: {tail}"
    );
}

#[test]
fn symbols_list_variants_and_fields_only_when_asked_and_show_docs() {
    let dir = TempProject::new("extras");
    dir.write(
        "src/lib.rs",
        "/// A shape.\npub enum Shape {\n    Dot,\n    Line,\n}\npub struct P {\n    pub x: i32,\n}\n/// Make one.\npub fn make() {}\n",
    );
    let state = State::new(ServerConfig::default());
    let list = |kinds: Option<BTreeMap<String, Vec<String>>>, docs: bool| {
        state
            .langtool_find_symbols(FindSymbolsParams {
                implicit_path: Some(dir.path_str()),
                kinds,
                docs: Some(docs),
                ..FindSymbolsParams::default()
            })
            .unwrap()
    };

    let out = list(None, false);
    // Rows do not carry the words `variant` and `field`, so look for the members themselves.
    assert!(
        out.contains("pub enum Shape") && !out.contains("Dot") && !out.contains("x: i32"),
        "{out}"
    );
    // A symbol's range starts at its doc comment.
    assert!(
        out.contains("1..5\tpub enum Shape") && out.contains("9..10\tpub fn make()"),
        "{out}"
    );

    let out = list(kinds("rust", &["variant", "field"]), false);
    assert!(
        out.contains("3\tDot (in Shape)") && out.contains("7\tpub x: i32 (in P)"),
        "{out}"
    );

    let out = list(None, true);
    assert!(
        out.contains("\tA shape.") && out.contains("\tMake one."),
        "{out}"
    );
}
