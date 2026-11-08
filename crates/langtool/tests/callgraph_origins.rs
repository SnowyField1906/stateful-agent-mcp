//! Calls without a definition in the scan are placed by the imports of their file: no list of
//! well-known names, only manifests, `.gitignore` and the project directory.

use stateful_agent_common::ServerConfig;
use stateful_agent_langtool::tools::ViewCallgraphParams;
use stateful_agent_testkit::TempDir;

use crate::support::Tools;

fn callgraph(dir: &TempDir, symbol: &str, direction: &str, show_external: bool) -> String {
    Tools::new(ServerConfig::default())
        .langtool_view_callgraph(ViewCallgraphParams {
            symbol: symbol.into(),
            implicit_path: Some(dir.path_str()),
            direction: Some(direction.parse().unwrap()),
            show_external: Some(show_external),
            ..ViewCallgraphParams::default()
        })
        .unwrap()
}

/// Whether an interpreter that lists its standard library (3.10 or newer) is installed.
fn python_knows_stdlib() -> bool {
    ["python3", "python"].iter().any(|exe| {
        std::process::Command::new(exe)
            .args([
                "-I",
                "-S",
                "-c",
                "import sys; sys.exit(0 if hasattr(sys, 'stdlib_module_names') else 1)",
            ])
            .status()
            .is_ok_and(|status| status.success())
    })
}

#[test]
fn rust_calls_are_placed_by_imports_manifest_and_gitignore() {
    let dir = TempDir::new("origins-rust");
    dir.write(
        "Cargo.toml",
        concat!(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n",
            "[dependencies]\n",
            "serde_json = \"1\"\n",
            "inner = { path = \"crates/inner\" }\n",
            "vendored = { path = \"vendor/vendored\" }\n",
            "faraway = { path = \"../faraway\" }\n",
        ),
    );
    dir.write(".gitignore", "vendor/\n");
    dir.write(
        "src/lib.rs",
        concat!(
            "use std::fs;\n",
            "use serde_json::from_str;\n",
            "use inner::tidy;\n",
            "use vendored::polish;\n",
            "use faraway::reach;\n",
            "\n",
            "pub fn run() {\n",
            "    from_str(\"1\");\n",
            "    serde_json::to_string(&1);\n",
            "    fs::read(\"a\");\n",
            "    std::mem::drop(1);\n",
            "    tidy();\n",
            "    polish();\n",
            "    reach();\n",
            "    drop(1);\n",
            "    value.frob();\n",
            "}\n",
        ),
    );

    // std: a path through an import, a written `std::` path and the prelude; deps: a declared
    // crate, a path dependency in an ignored folder and one outside the project directory;
    // project: a path dependency inside the project that the scan did not reach.
    let out = callgraph(&dir, "run", "outgoing", false);
    assert!(
        out.contains(
            "  (external: 3 std, 1 unresolved; deps: serde_json, vendored, faraway; project: tidy)\n"
        ),
        "{out}"
    );

    let out = callgraph(&dir, "run", "outgoing", true);
    for tag in [
        "from_str\t(dep serde_json)",
        "to_string\t(dep serde_json)",
        "read\t(std)",
        "tidy\t(project)",
        "polish\t(dep vendored)",
        "reach\t(dep faraway)",
        "drop\t(std)",
        "frob\t(unresolved)",
    ] {
        assert!(out.contains(tag), "missing {tag:?}: {out}");
    }
}

#[test]
fn an_import_from_a_dependency_never_links_to_a_same_named_project_function() {
    let dir = TempDir::new("origins-same-name");
    dir.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde_json = \"1\"\n",
    );
    dir.write("src/helpers.rs", "pub fn from_str() {}\n");
    dir.write(
        "src/lib.rs",
        "use serde_json::from_str;\npub fn run() { from_str(\"1\"); }\n",
    );
    dir.write(
        "src/go.rs",
        "use crate::helpers::from_str;\npub fn go() { from_str(); }\n",
    );

    let out = callgraph(&dir, "from_str", "incoming", false);
    assert!(out.contains("go\t"), "{out}");
    assert!(!out.contains("run"), "{out}");

    let out = callgraph(&dir, "run", "outgoing", false);
    assert!(out.contains("(external: deps: serde_json)"), "{out}");
    assert!(!out.contains("helpers"), "{out}");
}

#[test]
fn a_receiver_typed_from_the_standard_library_is_not_a_possible_project_method() {
    let dir = TempDir::new("origins-typed");
    dir.write(
        "src/lib.rs",
        concat!(
            "use std::collections::HashMap;\n",
            "pub struct Cache;\n",
            "impl Cache {\n",
            "    pub fn get(&self) {}\n",
            "}\n",
            "pub fn lookup(map: &HashMap<String, u32>) {\n",
            "    map.get(\"a\");\n",
            "}\n",
        ),
    );
    let out = callgraph(&dir, "lookup", "outgoing", false);
    assert!(!out.contains("Cache"), "{out}");
    assert!(out.contains("(external: 1 std)"), "{out}");
}

#[test]
fn a_prelude_receiver_is_not_a_possible_project_method() {
    // `Vec` is never imported, so only the project's own declarations can say it is not std. An
    // `impl From<..> for Vec<..>` gives it no methods of the project's and must not make it one.
    let dir = TempDir::new("origins-prelude");
    dir.write(
        "src/lib.rs",
        concat!(
            "pub struct Lineage;\n",
            "impl Lineage {\n",
            "    pub fn push(&self) {}\n",
            "    pub fn first(&self) {}\n",
            "}\n",
            "pub struct Params;\n",
            "impl From<Params> for Vec<String> {\n",
            "    fn from(_: Params) -> Self {\n",
            "        Vec::new()\n",
            "    }\n",
            "}\n",
            "pub fn collect(names: &mut Vec<String>) {\n",
            "    let mut parts: Vec<String> = Vec::new();\n",
            "    parts.push(String::new());\n",
            "    names.push(String::new());\n",
            "    names.first();\n",
            "}\n",
        ),
    );
    let out = callgraph(&dir, "collect", "outgoing", false);
    assert!(!out.contains("Lineage"), "{out}");
    assert!(!out.contains("project"), "{out}");
    assert!(out.contains("std"), "{out}");

    // The other direction agrees: nothing in `collect` calls `Lineage::push`.
    let out = callgraph(&dir, "push", "incoming", false);
    assert!(!out.contains("collect"), "{out}");
}

#[test]
fn a_primitive_or_slice_receiver_is_not_a_possible_project_method() {
    // `&str` and `&[u8]` are never imported and never declared, so they are the standard
    // library's like `Vec`: a project method that shares the name is not a target.
    let dir = TempDir::new("origins-primitive");
    dir.write(
        "src/lib.rs",
        concat!(
            "pub struct Lineage;\n",
            "impl Lineage {\n",
            "    pub fn starts_with(&self) {}\n",
            "    pub fn len(&self) {}\n",
            "}\n",
            "pub fn check(name: &str, bytes: &[u8]) {\n",
            "    name.starts_with(\"a\");\n",
            "    bytes.len();\n",
            "}\n",
        ),
    );
    let out = callgraph(&dir, "check", "outgoing", false);
    assert!(!out.contains("Lineage"), "{out}");
    assert!(out.contains("(external: 2 std)"), "{out}");
}

#[test]
fn a_match_guard_uses_a_parameter_without_binding_it() {
    // `name` is only read in the guard, so it is still the `&str` parameter.
    let dir = TempDir::new("origins-guard");
    dir.write(
        "src/lib.rs",
        concat!(
            "pub struct Lineage;\n",
            "impl Lineage {\n",
            "    pub fn starts_with(&self) {}\n",
            "}\n",
            "pub fn check(name: &str, kind: Option<u8>) {\n",
            "    match kind {\n",
            "        None if name.starts_with(\"a\") => {}\n",
            "        _ => {}\n",
            "    }\n",
            "}\n",
        ),
    );
    let out = callgraph(&dir, "check", "outgoing", false);
    assert!(!out.contains("Lineage"), "{out}");
    assert!(out.contains("(external: 1 std)"), "{out}");
}

#[test]
fn a_project_trait_implemented_for_a_primitive_type_keeps_its_edge() {
    let dir = TempDir::new("origins-primitive-trait");
    dir.write(
        "src/lib.rs",
        concat!(
            "pub trait Shout {\n",
            "    fn shout(&self);\n",
            "}\n",
            "impl Shout for str {\n",
            "    fn shout(&self) {}\n",
            "}\n",
            "pub fn typed(text: &str) {\n",
            "    text.shout();\n",
            "}\n",
        ),
    );
    let out = callgraph(&dir, "typed", "outgoing", false);
    assert!(out.contains("shout"), "{out}");
}

#[test]
fn a_function_imported_from_a_project_module_is_not_a_same_named_one_of_another_module() {
    let dir = TempDir::new("origins-project-import");
    dir.write(
        "src/lib.rs",
        concat!(
            "mod one;\n",
            "mod two;\n",
            "use crate::one::run;\n",
            "pub fn go() {\n",
            "    run();\n",
            "}\n",
        ),
    );
    dir.write("src/one.rs", "pub fn run() {}\n");
    dir.write("src/two.rs", "pub fn run() {}\n");
    let out = callgraph(&dir, "go", "outgoing", false);
    assert!(out.contains("src/one.rs"), "{out}");
    assert!(!out.contains("src/two.rs"), "{out}");
}

#[test]
fn a_re_exported_function_keeps_every_candidate() {
    // `api` re-exports `reexported` and defines nothing, so the import cannot rule a module out.
    let dir = TempDir::new("origins-reexport");
    dir.write(
        "src/lib.rs",
        concat!(
            "mod api;\n",
            "mod inner;\n",
            "use crate::api::reexported;\n",
            "pub fn go() {\n",
            "    reexported();\n",
            "}\n",
        ),
    );
    dir.write("src/api.rs", "pub use crate::inner::reexported;\n");
    dir.write("src/inner.rs", "pub fn reexported() {}\n");
    let out = callgraph(&dir, "go", "outgoing", false);
    assert!(out.contains("src/inner.rs"), "{out}");
}

#[test]
fn a_project_trait_implemented_for_a_prelude_type_keeps_its_edge() {
    let dir = TempDir::new("origins-prelude-trait");
    dir.write(
        "src/lib.rs",
        concat!(
            "pub trait Shout {\n",
            "    fn shout(&self);\n",
            "}\n",
            "impl Shout for String {\n",
            "    fn shout(&self) {}\n",
            "}\n",
            "pub fn typed(text: &String) {\n",
            "    text.shout();\n",
            "}\n",
            "pub fn untyped() {\n",
            "    value.shout();\n",
            "}\n",
        ),
    );
    // The method is the project's own on that very receiver type, so calling it on a `String`
    // elsewhere does not turn a call on a value of unknown type into a std call.
    let out = callgraph(&dir, "typed", "outgoing", false);
    assert!(out.contains("shout"), "{out}");
    let out = callgraph(&dir, "untyped", "outgoing", false);
    assert!(out.contains("shout"), "{out}");
}

#[test]
fn python_calls_are_placed_by_imports_and_the_interpreter() {
    let dir = TempDir::new("origins-python");
    dir.write("pkg/util.py", "def work():\n    pass\n");
    dir.write(
        "src/app.py",
        concat!(
            "from pkg import util\n",
            "from . import sibling\n",
            "import os\n",
            "import numpy as np\n",
            "\n",
            "def app_main():\n",
            "    util.work()\n",
            "    sibling.go()\n",
            "    os.path.join(\"a\")\n",
            "    np.array([1])\n",
            "    len([])\n",
        ),
    );
    dir.write(
        "src/star.py",
        "from helpers import *\n\ndef star_main():\n    mystery()\n",
    );

    let out = callgraph(&dir, "app_main", "outgoing", false);
    if python_knows_stdlib() {
        assert!(
            out.contains("(external: 2 std; deps: numpy; project: work, go)"),
            "{out}"
        );
    } else {
        // Without an interpreter, packages outside the project cannot be told apart.
        assert!(out.contains("project: work, go)"), "{out}");
        assert!(out.contains("unresolved"), "{out}");
    }

    // A wildcard import may bring in any name, so nothing is assumed to be a builtin.
    let out = callgraph(&dir, "star_main", "outgoing", false);
    assert!(out.contains("(external: 1 unresolved)"), "{out}");
}

#[test]
fn an_untyped_method_named_like_a_std_method_called_elsewhere_is_not_a_project_method_call() {
    let stack = concat!(
        "pub struct Stack;\n",
        "impl Stack {\n",
        "    pub fn push(&self) {}\n",
        "}\n",
        "pub fn run() {\n",
        "    value.push(1);\n",
        "}\n",
    );
    // Nothing shows that `value.push()` is not `Stack::push`.
    let plain = TempDir::new("origins-untyped-plain");
    plain.write("src/lib.rs", stack);
    let out = callgraph(&plain, "run", "outgoing", false);
    assert!(out.contains("Stack"), "{out}");

    // A `Vec` receiver calls `push` elsewhere, so the untyped call is left unresolved.
    let typed = TempDir::new("origins-untyped-typed");
    typed.write(
        "src/lib.rs",
        &format!(
            "{stack}pub fn fill(names: &mut Vec<String>) {{\n    names.push(String::new());\n}}\n"
        ),
    );
    let out = callgraph(&typed, "run", "outgoing", false);
    assert!(!out.contains("Stack"), "{out}");
    assert!(out.contains("(external: 1 unresolved)"), "{out}");

    // Asking about `push` itself still shows its untyped callers.
    let out = callgraph(&typed, "push", "incoming", false);
    assert!(out.contains("run\t"), "{out}");
}

fn callgraph_at(dir: &TempDir, symbol: &str, path: &str) -> String {
    Tools::in_project(dir, ServerConfig::default())
        .langtool_view_callgraph(ViewCallgraphParams {
            symbol: symbol.into(),
            implicit_path: Some(path.to_string()),
            direction: Some("outgoing".parse().unwrap()),
            ..ViewCallgraphParams::default()
        })
        .unwrap()
}

#[test]
fn a_sibling_project_is_a_dependency_but_a_workspace_member_is_not() {
    let dir = TempDir::new("origins-boundary");
    dir.write(
        "app/Cargo.toml",
        concat!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n",
            "[dependencies]\nsibling = { path = \"../sibling\" }\n",
        ),
    );
    dir.write(
        "app/src/lib.rs",
        "use sibling::helper;\npub fn run() {\n    helper();\n}\n",
    );
    dir.write("app/src/own.rs", "pub fn helper() {}\n");
    dir.write(
        "sibling/Cargo.toml",
        "[package]\nname = \"sibling\"\nversion = \"0.1.0\"\n",
    );
    dir.write("sibling/src/lib.rs", "pub fn helper() {}\n");

    // The sibling lies inside the registered directory but is not part of the app.
    let out = callgraph_at(&dir, "run", "app");
    assert!(out.contains("(external: deps: sibling)"), "{out}");

    // When it is scanned too, the call reaches its function and not the app's namesake.
    let out = callgraph_at(&dir, "run", ".");
    assert!(out.contains("sibling/src/lib.rs"), "{out}");
    assert!(!out.contains("app/src/own.rs"), "{out}");

    // The members of a workspace are one project.
    let ws = TempDir::new("origins-workspace");
    ws.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/a\", \"crates/b\"]\n",
    );
    ws.write(
        "crates/a/Cargo.toml",
        concat!(
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n\n",
            "[dependencies]\nb = { path = \"../b\" }\n",
        ),
    );
    ws.write(
        "crates/a/src/lib.rs",
        "use b::tidy;\npub fn run() {\n    tidy();\n}\n",
    );
    ws.write(
        "crates/b/Cargo.toml",
        "[package]\nname = \"b\"\nversion = \"0.1.0\"\n",
    );
    ws.write("crates/b/src/lib.rs", "pub fn tidy() {}\n");
    let out = callgraph_at(&ws, "run", "crates/a");
    assert!(out.contains("(external: project: tidy)"), "{out}");
}
