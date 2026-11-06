use agent_mcp::ServerConfig;
use agent_mcp::langtool::tools::ViewCallgraphParams;
use agent_mcp::server::State;

use crate::LIB;
use crate::common::TempProject;

#[test]
fn callgraph_resolves_paths_hides_noise_and_suggests_names() {
    let dir = TempProject::new("calls");
    dir.write("src/lib.rs", LIB);
    dir.write(
        "tests/store.rs",
        "fn check() { let s = Store::new(); s.put(\"a\"); }\n",
    );
    let state = State::new(ServerConfig::default());
    let call = |symbol: &str, direction: &str, include_tests: bool, show_external: bool| {
        state.langtool_view_callgraph(ViewCallgraphParams {
            symbol: symbol.into(),
            implicit_path: Some(dir.path_str()),
            direction: Some(direction.into()),
            include_tests: Some(include_tests),
            show_external: Some(show_external),
            ..ViewCallgraphParams::default()
        })
    };

    // `Store::new()` reaches the method; the `Some(..)`, `to_string` and `map` calls are not
    // listed as functions, only summarized.
    let out = call("put", "outgoing", false, false).unwrap();
    assert!(out.contains("(external: to_string, map)"), "{out}");
    assert!(!out.contains("Some"), "{out}");

    let out = call("put", "outgoing", false, true).unwrap();
    assert!(out.contains("to_string\t(external)"), "{out}");

    // The test calling `new` and `put` is hidden by default.
    let out = call("new", "incoming", false, false).unwrap();
    assert!(out.contains("open") && !out.contains("check"), "{out}");
    let out = call("new", "incoming", true, false).unwrap();
    assert!(out.contains("check"), "{out}");

    // `s.put(..)` is a method call: `setup` calls the method, never a free function.
    let out = call("put", "incoming", false, false).unwrap();
    assert!(out.contains("setup"), "{out}");

    let out = call("setu", "both", false, false).unwrap();
    assert!(
        out.contains("no function `setu` found; similar names: setup"),
        "{out}"
    );
}

#[test]
fn view_callgraph_accepts_qualified_symbols() {
    let tmp = TempProject::new("code-calls-qualified");
    tmp.write(
        "src/a.rs",
        "struct A;\nstruct B;\nimpl A {\n    fn new() -> A { A }\n}\nimpl B {\n    fn new() -> B { B }\n}\nfn main() { A::new(); B::new(); }\n",
    );
    let state = State::new(ServerConfig::default());
    let call = |symbol: &str| {
        state.langtool_view_callgraph(ViewCallgraphParams {
            symbol: symbol.into(),
            implicit_path: Some(tmp.path_str()),
            direction: Some("incoming".into()),
            ..ViewCallgraphParams::default()
        })
    };

    for symbol in ["B::new", "B.new", "B/new"] {
        let out = call(symbol).unwrap();
        assert!(out.starts_with("new (in B)\t"), "{symbol}: {out}");
        assert!(!out.contains("(in A)"), "{symbol}: {out}");
        assert!(out.contains("  main\t"), "{symbol}: {out}");
    }
    let out = call("new").unwrap();
    assert!(out.contains("(in A)") && out.contains("(in B)"), "{out}");
    let out = call("C::new").unwrap();
    assert!(out.contains("no function `C::new` found"), "{out}");
}

#[test]
fn view_callgraph_shows_callees_and_callers_across_files() {
    let tmp = TempProject::new("code-calls");
    tmp.write(
        "src/a.rs",
        "fn a() {\n    b();\n    ext();\n}\n\nfn b() {\n    c();\n}\n",
    );
    tmp.write("src/c.rs", "fn c() {\n    a();\n}\n");
    tmp.write("m.py", "def g():\n    pass\n\ndef f():\n    g()\n");
    tmp.write("notes.txt", "a b c\n");
    let state = State::new(ServerConfig::default());
    let calls = |symbol: &str, f: &dyn Fn(&mut ViewCallgraphParams)| {
        let mut p = ViewCallgraphParams {
            symbol: symbol.into(),
            implicit_path: Some(tmp.path_str()),
            ..Default::default()
        };
        f(&mut p);
        state.langtool_view_callgraph(p)
    };

    // Callees, with calls that have no definition marked external.
    let out = calls("a", &|p| p.direction = Some("outgoing".into())).unwrap();
    assert_eq!(
        out, "a\tsrc/a.rs:1..4\noutgoing:\n  b\tsrc/a.rs:6..8 @2\n  (external: ext)\n",
        "unexpected output: {out}"
    );

    // Both directions by default; the caller is in another file.
    let both = calls("a", &|_| {}).unwrap();
    assert!(
        both.contains("outgoing:") && both.contains("incoming:\n  c\tsrc/c.rs:1..3 @2\n"),
        "unexpected output: {both}"
    );

    // Tree depth follows the chain until it leads back.
    let deep = calls("a", &|p| {
        p.direction = Some("outgoing".into());
        p.tree_depth = Some(3);
    })
    .unwrap();
    assert!(
        deep.contains("    c\tsrc/c.rs:1..3 @7\n") && deep.contains("(cycle)"),
        "unexpected output: {deep}"
    );

    // Python, and narrowing by file_path or line.
    let py = calls("g", &|p| p.direction = Some("incoming".into())).unwrap();
    assert!(py.contains("f\tm.py:4..5 @5"), "unexpected output: {py}");
    assert!(
        calls("a", &|p| p.file_path = Some("src/c.rs".into()))
            .unwrap()
            .contains("no function")
    );
    assert!(
        calls("b", &|p| p.line = Some(7))
            .unwrap()
            .contains("b\tsrc/a.rs:6..8")
    );
    assert!(
        calls("b", &|p| p.line = Some(1))
            .unwrap()
            .contains("no function")
    );

    assert!(calls("zzz", &|_| {}).unwrap().contains("no function"));
    let err = calls(" ", &|_| {}).unwrap_err();
    assert!(
        err.to_string().contains("symbol"),
        "unexpected error: {err}"
    );
    let err = calls("a", &|p| p.direction = Some("sideways".into())).unwrap_err();
    assert!(
        err.to_string().contains("direction"),
        "unexpected error: {err}"
    );
}

#[test]
fn resolves_rust_method_receivers_from_types_and_constructors() {
    let dir = TempProject::new("typed-method-calls");
    dir.write(
        "src/lib.rs",
        include_str!("../fixtures/rust_method_receivers.rs"),
    );
    let state = State::new(ServerConfig::default());
    let out = state
        .langtool_view_callgraph(ViewCallgraphParams {
            symbol: "put".into(),
            implicit_path: Some(dir.path_str()),
            direction: Some("incoming".into()),
            ..ViewCallgraphParams::default()
        })
        .unwrap();

    let (store, queue) = out.split_once("put (in Queue)").unwrap();
    assert!(
        store.contains("  annotated\t")
            && store.contains("  aliased\t")
            && store.contains("  constructed\t")
            && store.contains("  tuple_constructed\t")
            && store.contains("  receiver_is_self (in Store)\t")
            && store.contains("  built_with_self (in Store)\t"),
        "{out}"
    );
    assert!(
        store.contains("  ? unknown\t")
            && store.contains("  ? wrapped\t")
            && store.contains("  ? shadowed_pattern\t")
            && !store.contains("? annotated")
            && !store.contains("? aliased"),
        "{out}"
    );
    assert!(
        queue.contains("  typed_local\t")
            && !queue.contains("? typed_local")
            && queue.contains("  unit_struct_value\t")
            && !queue.contains("? unit_struct_value"),
        "{out}"
    );
    assert!(
        queue.contains("  ? unknown\t")
            && queue.contains("  ? wrapped\t")
            && queue.contains("  ? shadowed_pattern\t"),
        "{out}"
    );

    let outgoing = state
        .langtool_view_callgraph(ViewCallgraphParams {
            symbol: "wrapped".into(),
            implicit_path: Some(dir.path_str()),
            direction: Some("outgoing".into()),
            ..ViewCallgraphParams::default()
        })
        .unwrap();
    assert!(
        outgoing.contains("? put (in Store)")
            && outgoing.contains("? put (in Queue)")
            && !outgoing.contains("(external: put)"),
        "{outgoing}"
    );

    for symbol in ["annotated", "receiver_is_self", "built_with_self"] {
        let outgoing = state
            .langtool_view_callgraph(ViewCallgraphParams {
                symbol: symbol.into(),
                implicit_path: Some(dir.path_str()),
                direction: Some("outgoing".into()),
                ..ViewCallgraphParams::default()
            })
            .unwrap();
        assert!(outgoing.contains("put (in Store)"), "{outgoing}");
        assert!(!outgoing.contains("(in Queue)"), "{outgoing}");
        assert!(!outgoing.contains("? put"), "{outgoing}");
    }

    let outgoing = state
        .langtool_view_callgraph(ViewCallgraphParams {
            symbol: "match_shadow".into(),
            implicit_path: Some(dir.path_str()),
            direction: Some("outgoing".into()),
            ..ViewCallgraphParams::default()
        })
        .unwrap();
    assert!(outgoing.contains("put (in Store)"), "{outgoing}");
    assert!(outgoing.contains("? put (in Queue)"), "{outgoing}");

    let outgoing = state
        .langtool_view_callgraph(ViewCallgraphParams {
            symbol: "for_shadow".into(),
            implicit_path: Some(dir.path_str()),
            direction: Some("outgoing".into()),
            ..ViewCallgraphParams::default()
        })
        .unwrap();
    assert!(outgoing.contains("? put (in Store)"), "{outgoing}");
    assert!(outgoing.contains("? put (in Queue)"), "{outgoing}");

    for symbol in ["if_let_shadow", "while_let_shadow"] {
        let outgoing = state
            .langtool_view_callgraph(ViewCallgraphParams {
                symbol: symbol.into(),
                implicit_path: Some(dir.path_str()),
                direction: Some("outgoing".into()),
                ..ViewCallgraphParams::default()
            })
            .unwrap();
        assert!(outgoing.contains("? put (in Store)"), "{outgoing}");
        assert!(outgoing.contains("? put (in Queue)"), "{outgoing}");
    }
}

#[test]
fn method_calls_on_other_values_are_not_self_cycles() {
    let dir = TempProject::new("cycles");
    dir.write(
        "src/lib.rs",
        "struct A;\nstruct B;\nimpl A {\n    fn go(&self, b: &B) { b.go(); }\n}\nimpl B {\n    fn go(&self) {}\n}\n",
    );
    let state = State::new(ServerConfig::default());
    let out = state
        .langtool_view_callgraph(ViewCallgraphParams {
            symbol: "go".into(),
            implicit_path: Some(dir.path_str()),
            tree_depth: Some(3),
            ..ViewCallgraphParams::default()
        })
        .unwrap();
    assert!(!out.contains("(cycle)"), "{out}");
    assert!(out.contains("go (in B)"), "{out}");
}

#[test]
fn same_named_rust_receiver_types_remain_possible() {
    let dir = TempProject::new("ambiguous-receivers");
    dir.write(
        "src/lib.rs",
        include_str!("../fixtures/rust_ambiguous_receivers.rs"),
    );
    let state = State::new(ServerConfig::default());
    let out = state
        .langtool_view_callgraph(ViewCallgraphParams {
            symbol: "parse".into(),
            implicit_path: Some(dir.path_str()),
            direction: Some("incoming".into()),
            ..ViewCallgraphParams::default()
        })
        .unwrap();
    assert_eq!(out.matches("? ambiguous").count(), 2, "{out}");
}
