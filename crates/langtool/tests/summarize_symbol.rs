//! `langtool_summarize_symbol` reads a folder that holds several projects one project at a time: a
//! name that another project writes the same is no use of the definition.

use stateful_agent_common::{PageParams, ServerConfig};
use stateful_agent_langtool::engine::lens::store::FactStore;
use stateful_agent_langtool::tools::{SummarizeSymbolParams, summarize_symbol};
use stateful_agent_testkit::TempDir;

/// Two projects below one folder, each with its own `Cargo.toml`. A `Fixture` is declared in `a`;
/// `a/src/app.rs` and `b/src/bench.rs` both write that name in a signature.
fn projects() -> TempDir {
    let dir = TempDir::new("summarize_symbol");
    for project in ["a", "b"] {
        dir.write(
            &format!("{project}/Cargo.toml"),
            &format!("[package]\nname = \"{project}\"\nversion = \"0.1.0\"\n"),
        );
    }
    dir.write(
        "a/src/kit.rs",
        "pub struct Fixture {\n    pub id: u32,\n}\n",
    );
    dir.write("a/src/app.rs", "pub fn run(fixture: Fixture) {}\n");
    dir.write("b/src/bench.rs", "pub fn bench(fixture: Fixture) {}\n");
    dir
}

/// The answer for `symbol` when the folder `dir` is scanned by its absolute path, with no project
/// registered: the shown paths are relative to `dir`.
fn answer(dir: &TempDir, symbol: &str) -> String {
    summarize_symbol::summarize_symbol(
        None,
        &ServerConfig::default(),
        &FactStore::new(64),
        SummarizeSymbolParams {
            symbol: symbol.to_string(),
            implicit_path: Some(dir.path_str()),
            ..Default::default()
        },
    )
    .unwrap()
}

/// One project in which `Hub` is used by six functions, one per file.
fn hub() -> TempDir {
    let dir = TempDir::new("summarize_symbol_hub");
    dir.write(
        "Cargo.toml",
        "[package]\nname = \"hub\"\nversion = \"0.1.0\"\n",
    );
    dir.write("src/hub.rs", "pub struct Hub {\n    pub id: u32,\n}\n");
    for n in 1..=6 {
        dir.write(
            &format!("src/user{n}.rs"),
            &format!("pub fn use_hub{n}(hub: Hub) {{}}\n"),
        );
    }
    dir
}

/// The answer for `Hub` at zoom `low` (four rows per list), with `entries` when given.
fn hub_answer(dir: &TempDir, entries: Option<&str>) -> String {
    summarize_symbol::summarize_symbol(
        None,
        &ServerConfig::default(),
        &FactStore::new(64),
        SummarizeSymbolParams {
            symbol: "Hub".to_string(),
            implicit_path: Some(dir.path_str()),
            zoom: Some("low".to_string()),
            page: PageParams {
                entries: entries.map(|text| text.parse().unwrap()),
            },
            ..Default::default()
        },
    )
    .unwrap()
}

/// How many of `use_hub1..use_hub6` the answer names.
fn users(out: &str) -> usize {
    (1..=6)
        .filter(|n| out.contains(&format!("use_hub{n}")))
        .count()
}

#[test]
fn a_definition_is_used_by_its_own_project_only() {
    let out = answer(&projects(), "Fixture");
    assert!(out.contains("a/src/app.rs"), "{out}");
    assert!(!out.contains("b/src/bench.rs"), "{out}");
}

#[test]
fn a_cut_list_names_the_entries_that_read_the_rest() {
    let dir = hub();
    let out = hub_answer(&dir, None);
    assert!(out.contains("used by (6):"), "{out}");
    assert_eq!(users(&out), 4, "{out}");
    assert!(
        out.contains("… 2 more (section=used by, entries=5..6)"),
        "{out}"
    );
}

#[test]
fn entries_show_the_rows_of_the_lists_from_that_position() {
    let dir = hub();
    let first = hub_answer(&dir, None);
    let rest = hub_answer(&dir, Some("5..6"));
    assert!(rest.contains("used by (6):"), "{rest}");
    assert_eq!(users(&rest), 2, "{rest}");
    assert!(!rest.contains(" more (section="), "{rest}");
    // The two pages together name every user, and no user twice.
    let both = format!("{first}\n{rest}");
    assert_eq!(users(&both), 6, "{both}");
    for n in 1..=6 {
        let name = format!("use_hub{n}");
        let rows = |text: &str| text.lines().filter(|line| line.contains(&name)).count();
        assert_eq!(rows(&first) + rows(&rest), 1, "{name}\n{first}\n{rest}");
    }
}

#[test]
fn users_in_test_code_are_counted_apart() {
    let dir = TempDir::new("summarize_symbol_tests");
    dir.write(
        "Cargo.toml",
        "[package]\nname = \"kit\"\nversion = \"0.1.0\"\n",
    );
    dir.write("src/kit.rs", "pub struct Kit {\n    pub id: u32,\n}\n");
    dir.write("src/app.rs", "pub fn run(kit: Kit) {}\n");
    dir.write("tests/check.rs", "pub fn check(kit: Kit) {}\n");
    let out = answer(&dir, "Kit");
    assert!(out.contains("used by (2, 1 in tests):"), "{out}");
}

#[test]
fn a_section_shows_that_list_only() {
    let dir = hub();
    let ask = |section: &str| {
        summarize_symbol::summarize_symbol(
            None,
            &ServerConfig::default(),
            &FactStore::new(64),
            SummarizeSymbolParams {
                symbol: "Hub".to_string(),
                implicit_path: Some(dir.path_str()),
                zoom: Some("low".to_string()),
                section: Some(section.to_string()),
                ..Default::default()
            },
        )
    };
    let out = ask("used by").unwrap();
    assert!(out.contains("used by (6):"), "{out}");
    assert!(!out.contains("uses"), "{out}");
    assert!(!out.contains("sig:"), "{out}");
    assert!(ask("callers").is_err());
}

#[test]
fn entries_past_the_end_say_so() {
    let out = hub_answer(&hub(), Some("20..30"));
    assert!(
        out.contains("used by (6):\n  nothing at these entries"),
        "{out}"
    );
}
