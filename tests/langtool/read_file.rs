use agent_mcp::ServerConfig;
use agent_mcp::common::Range;
use agent_mcp::langtool::tools::{ReadFileEntry, ReadFileParams};
use agent_mcp::server::State;

use crate::common::TempProject;

const DOCS_SRC: &str = concat!(
    "/// Stores things.\n",
    "#[derive(Debug)]\n",
    "pub struct Store;\n",
    "impl Store {\n",
    "    /// Put a key.\n",
    "    pub fn put(&self) {\n",
    "        let a = 1;\n",
    "        let b = 2;\n",
    "    }\n",
    "    pub fn get(&self) {\n",
    "        let c = 3;\n",
    "        let d = 4;\n",
    "    }\n",
    "}\n",
);

#[test]
fn read_file_params_accept_files_as_a_json_string_and_hints_otherwise() {
    let ok: ReadFileParams =
        serde_json::from_value(serde_json::json!({ "files": "[{\"file_path\": \"a.txt\"}]" }))
            .unwrap();
    assert_eq!(ok.files.len(), 1);

    let err = serde_json::from_value::<ReadFileParams>(serde_json::json!({ "files": "not json" }))
        .unwrap_err();
    assert!(
        err.to_string().contains("array of objects"),
        "unexpected error: {err}"
    );

    let parsed: ReadFileParams = serde_json::from_value(
        serde_json::json!({ "files": [{ "file_path": "a.txt", "numbered": false }] }),
    )
    .unwrap();
    assert_eq!(parsed.files[0].numbered, Some(false));
}

#[test]
fn read_file_isolates_errors_reads_symbols_and_pages_folds_by_rows() {
    let dir = TempProject::new("read");
    dir.write("a.rs", DOCS_SRC);
    dir.write("big.txt", &"line\n".repeat(450));
    let state = State::new(ServerConfig::default());
    let abs = |name: &str| dir.root.join(name).to_string_lossy().into_owned();
    let req = |name: &str| ReadFileEntry {
        file_path: abs(name),
        ..ReadFileEntry::default()
    };
    let read = |files: Vec<ReadFileEntry>| {
        state.langtool_read_file(ReadFileParams {
            files,
            at_implicit_id: None,
        })
    };

    // A file that fails is reported in place; the others are still read.
    let out = read(vec![req("missing.rs"), req("a.rs")]).unwrap();
    assert!(
        out.contains("missing.rs: error:") && out.contains("pub struct Store"),
        "{out}"
    );
    assert!(read(vec![req("missing.rs")]).is_err());

    // A symbol read starts at the doc comment and names its scope.
    let mut item = req("a.rs");
    item.symbol = Some("Store::put".into());
    let out = read(vec![item]).unwrap();
    assert!(out.contains("[5..9 of 14] fn put (in Store)"), "{out}");
    assert!(
        out.contains("5\t    /// Put a key.") && !out.contains("fn get"),
        "{out}"
    );

    // ... and combines with folding.
    let mut item = req("a.rs");
    item.symbol = Some("put".into());
    item.fold_level = Some(0.into());
    let out = read(vec![item]).unwrap();
    assert!(out.contains("6..9\t    pub fn put(&self) { ... }"), "{out}");

    let mut item = req("a.rs");
    item.symbol = Some("nope".into());
    let err = read(vec![item]).unwrap_err().to_string();
    assert!(err.contains("no definition `nope`"), "{err}");

    let mut item = req("a.rs");
    item.symbol = Some("Store::putt".into());
    let err = read(vec![item]).unwrap_err().to_string();
    assert!(err.contains("no definition `Store::putt`"), "{err}");
    assert!(err.contains("similar names: put"), "{err}");

    let mut item = req("big.txt");
    item.symbol = Some("nope".into());
    let err = read(vec![item]).unwrap_err().to_string();
    let supported = agent_mcp::langtool::engine::syntax::registry::supported_file_types();
    assert!(err.contains(&format!("supported: {supported}")), "{err}");

    let mut item = req("a.rs");
    item.symbol = Some("put".into());
    item.lines = Some(Range::new(1, 2));
    let err = read(vec![item]).unwrap_err().to_string();
    assert!(err.contains("cannot be combined"), "{err}");

    // Folded views are capped in rows and continue from the last shown line.
    let mut item = req("big.txt");
    item.fold_level = Some(5.into());
    let out = read(vec![item]).unwrap();
    assert!(
        out.contains("[1..400 of 450] fold_level 5 indent next lines=401.."),
        "{out}"
    );
    let mut item = req("big.txt");
    item.fold_level = Some(5.into());
    item.lines = Some("401..".parse().unwrap());
    let out = read(vec![item]).unwrap();
    assert!(
        out.contains("[401..450 of 450]") && !out.contains("next lines"),
        "{out}"
    );
}

#[test]
fn read_file_symbol_accepts_the_full_qualifier_path_and_all_separators() {
    let dir = TempProject::new("read-qualified");
    dir.write(
        "a.rs",
        "mod east {\n    impl Token {\n        fn parse() {}\n    }\n}\nmod west {\n    impl Token {\n        fn parse() {}\n    }\n}\n",
    );
    let state = State::new(ServerConfig::default());
    let read = |symbol: &str| {
        state.langtool_read_file(ReadFileParams {
            files: vec![ReadFileEntry {
                file_path: dir.root.join("a.rs").to_string_lossy().into_owned(),
                symbol: Some(symbol.into()),
                ..ReadFileEntry::default()
            }],
            at_implicit_id: None,
        })
    };

    for symbol in [
        "east::Token::parse",
        "east.Token.parse",
        "east/Token/parse",
        "east::parse",
    ] {
        let out = read(symbol).unwrap();
        assert!(out.contains("3\t        fn parse() {}"), "{symbol}: {out}");
        assert!(!out.contains("8\t"), "{symbol}: {out}");
    }
    let both = read("Token::parse").unwrap();
    assert!(
        both.contains("fn parse (in east::Token)") && both.contains("fn parse (in west::Token)"),
        "{both}"
    );
    let err = read("west::east::parse").unwrap_err().to_string();
    assert!(err.contains("no definition `west::east::parse`"), "{err}");
}

#[test]
fn read_file_folds_deep_blocks_with_fold_level_and_pages_by_source_line() {
    let tmp = TempProject::new("code-depth");
    tmp.write(
        "a.rs",
        "struct P {\n    x: i32,\n}\n\nimpl P {\n    fn new() -> Self {\n        P { x: 0 }\n    }\n}\n",
    );
    tmp.write("notes.txt", "not code\n");
    let state = State::new(ServerConfig::default());
    let depth = |file: &str, f: &dyn Fn(&mut ReadFileEntry)| {
        let mut item = ReadFileEntry {
            file_path: tmp.root.join(file).to_string_lossy().into_owned(),
            fold_level: Some(1.into()),
            ..ReadFileEntry::default()
        };
        f(&mut item);
        state.langtool_read_file(ReadFileParams {
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
    assert!(one.contains("P { x: 0 }"), "unexpected output: {one}");
    let outline = depth("a.rs", &|p| p.fold_level = Some(0.into())).unwrap();
    assert!(
        outline.contains("fn new() -> Self { ... }") && !outline.contains("P { x: 0 }"),
        "unexpected output: {outline}"
    );

    // fold_level 2 shows the body; fold_level 0 folds the struct fields as well.
    let two = depth("a.rs", &|p| p.fold_level = Some(2.into())).unwrap();
    assert!(two.contains("P { x: 0 }"), "unexpected output: {two}");
    let zero = depth("a.rs", &|p| p.fold_level = Some(0.into())).unwrap();
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

    let text = depth("notes.txt", &|_| {}).unwrap();
    assert!(
        text.contains("fold_level 1 indent"),
        "unexpected output: {text}"
    );
}

#[test]
fn read_file_symbol_batch_renders_in_source_order() {
    let dir = TempProject::new("read-batch-symbols");
    dir.write(
        "src/lib.rs",
        "fn first() -> i32 { 1 }\nfn second() -> i32 { 2 }\nfn third() -> i32 { 3 }\n",
    );
    let state = State::new(ServerConfig::default());
    // Give symbols in reverse order: ["third", "first"]
    let out = state
        .langtool_read_file(ReadFileParams {
            files: vec![ReadFileEntry {
                file_path: dir.root.join("src/lib.rs").to_string_lossy().into_owned(),
                symbol: Some(vec!["third".to_string(), "first".to_string()].into()),
                ..ReadFileEntry::default()
            }],
            at_implicit_id: None,
        })
        .unwrap();

    let pos_first = out.find("fn first() -> i32 { 1 }").expect("first present");
    let pos_third = out.find("fn third() -> i32 { 3 }").expect("third present");
    assert!(pos_first < pos_third, "rendered in source order: {out}");
    assert!(!out.contains("fn second()"), "second not included: {out}");
}
