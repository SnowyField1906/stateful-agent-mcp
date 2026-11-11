//! `langtool_summarize_codebase` folds a folder of sibling doors that share one shape into a single
//! `doors:` row, and keeps fewer than four, or doors of another shape, as rows of their own.

use stateful_agent_common::ServerConfig;
use stateful_agent_langtool::tools::SummarizeCodebaseParams;
use stateful_agent_testkit::TempDir;

use crate::support::Tools;

/// The type every door mentions: the doors get dependencies and, being unused, no dependents.
const BASE: &str = "pub struct Base {\n    pub id: u32,\n}\n";

/// A door file that holds one `Params` struct named after `stem` and mentions `Base`.
fn params_door(stem: &str) -> String {
    let mut name = stem.to_string();
    name[..1].make_ascii_uppercase();
    format!("pub struct {name}Params {{\n    pub base: Base,\n}}\n")
}

/// A project with `src/base.rs` and one `Params` door per stem in `src/tools/`.
fn project(stems: &[&str]) -> TempDir {
    let dir = TempDir::new("summarize_doors");
    dir.write("src/base.rs", BASE);
    for stem in stems {
        dir.write(&format!("src/tools/{stem}.rs"), &params_door(stem));
    }
    dir
}

fn overview(dir: &TempDir) -> String {
    Tools::in_project(dir, ServerConfig::default())
        .langtool_summarize_codebase(SummarizeCodebaseParams::default())
        .unwrap()
}

/// The rows of the `doors:` section: the one after the label, then the indented ones.
fn doors(out: &str) -> Vec<&str> {
    let mut rows = Vec::new();
    let mut inside = false;
    for line in out.lines() {
        if let Some(first) = line.strip_prefix("doors:") {
            inside = true;
            rows.push(first.trim());
        } else if inside && line.starts_with("        ") {
            rows.push(line.trim());
        } else {
            inside = false;
        }
    }
    rows
}

#[test]
fn four_doors_of_one_shape_fold_to_one_row() {
    let stems = ["ant", "bee", "cat", "dog"];
    let out = overview(&project(&stems));
    let rows = doors(&out);

    assert_eq!(rows.len(), 1, "{out}");
    let row = rows[0];
    assert!(row.contains("4 files of one shape"), "{out}");
    assert!(row.contains("most used"), "{out}");
    assert!(row.contains("also"), "{out}");
    // The exemplar and the three others are all named; the folded row says which is which only by
    // position, because doors have no dependents to tell them apart.
    for stem in stems {
        assert!(
            row.contains(stem),
            "`{stem}` is missing from the row\n{out}"
        );
    }
}

#[test]
fn three_doors_of_one_shape_stay_separate_rows() {
    let stems = ["ant", "bee", "cat"];
    let out = overview(&project(&stems));
    let rows = doors(&out);

    assert_eq!(rows.len(), 3, "{out}");
    assert!(!out.contains("of one shape"), "{out}");
    for stem in stems {
        let file = format!("{stem}.rs");
        assert!(rows.iter().any(|row| row.contains(&file)), "{file}\n{out}");
    }
}

#[test]
fn a_door_of_another_shape_is_not_folded_into_the_family() {
    let stems = ["ant", "bee", "cat", "dog"];
    let dir = project(&stems);
    dir.write(
        "src/tools/odd.rs",
        "pub fn odd(base: &Base) -> Base {\n    Base { id: base.id }\n}\n",
    );
    let out = overview(&dir);
    let rows = doors(&out);

    assert_eq!(rows.len(), 2, "{out}");
    let family = rows.iter().find(|row| row.contains("of one shape"));
    let family = family.unwrap_or_else(|| panic!("no folded row\n{out}"));
    assert!(family.contains("4 files of one shape"), "{out}");
    assert!(!family.contains("odd"), "{out}");
    assert!(
        rows.iter().any(|row| row.contains("odd.rs")),
        "odd.rs has no row of its own\n{out}"
    );
}

/// A door file whose `Server` has `methods` exported methods that mention `Base`, called by no one.
fn server_project(methods: usize) -> TempDir {
    let dir = TempDir::new("summarize_owner");
    dir.write("src/base.rs", BASE);
    let body: String = (0..methods)
        .map(|at| format!("    pub fn m{at}(&self) -> Base {{\n        Base {{ id: 0 }}\n    }}\n"))
        .collect();
    dir.write(
        "src/server.rs",
        &format!("pub struct Server;\n\nimpl Server {{\n{body}}}\n"),
    );
    dir
}

#[test]
fn four_uncalled_methods_of_one_owner_are_counted_in_the_door_row() {
    let out = overview(&server_project(4));
    let rows = doors(&out);

    assert!(
        rows.iter().any(
            |row| row.contains("src/server.rs") && row.contains("Server 4 methods, no callers")
        ),
        "{out}"
    );
}

#[test]
fn three_uncalled_methods_are_not_counted() {
    let out = overview(&server_project(3));

    assert!(!out.contains("no callers"), "{out}");
}

const CRATES: usize = 7;
const FILES_PER_CRATE: usize = 12;

/// `crates/c0` .. `crates/c6`, each with twelve files in `src/`. Every file holds one function with
/// a long name, so that the names a collapsed row previews overflow the row.
fn workspace() -> TempDir {
    let dir = TempDir::new("summarize_tree");
    for krate in 0..CRATES {
        for file in 0..FILES_PER_CRATE {
            dir.write(
                &format!("crates/c{krate}/src/f{file}.rs"),
                &format!(
                    "pub fn a_rather_long_function_name_of_crate_{krate}_in_file_{file}() {{}}\n"
                ),
            );
        }
    }
    dir
}

/// The rows of the folder tree: the ones that start with a shown path and the collapsed ones.
fn tree_rows(out: &str) -> Vec<&str> {
    out.lines()
        .filter(|line| {
            line.starts_with("crates/") || (line.starts_with('+') && line.contains(" files in "))
        })
        .collect()
}

#[test]
fn every_folder_keeps_rows_of_its_own_when_the_files_outnumber_the_rows() {
    let out = overview(&workspace());
    let rows = tree_rows(&out);

    assert!(rows.len() <= 48, "{} rows\n{out}", rows.len());
    for krate in 0..CRATES {
        let folder = format!("crates/c{krate}/");
        assert!(
            rows.iter()
                .any(|row| row.starts_with(&folder) || row.contains(&format!("files in {folder}"))),
            "no row for {folder}\n{out}"
        );
    }
}

#[test]
fn a_collapsed_row_keeps_the_call_that_opens_its_folder() {
    let out = overview(&workspace());
    let rows = tree_rows(&out);

    for krate in 0..CRATES {
        let call = format!("> langtool_summarize_codebase implicit_path=crates/c{krate}/src");
        let row = rows
            .iter()
            .find(|row| row.contains(&format!("files in crates/c{krate}/src/")))
            .unwrap_or_else(|| panic!("no collapsed row for c{krate}\n{out}"));
        assert!(row.ends_with(&call), "{row}");
        assert!(row.chars().count() <= 120, "{row}");
    }
}

#[test]
fn a_folder_that_fits_is_listed_whole() {
    let dir = TempDir::new("summarize_tree_small");
    for file in 0..5 {
        dir.write(&format!("crates/a/src/f{file}.rs"), "pub fn f() {}\n");
    }
    let out = overview(&dir);

    assert_eq!(tree_rows(&out).len(), 5, "{out}");
    assert!(!out.contains("files in"), "{out}");
}

#[test]
fn functions_inside_a_test_module_count_in_the_test_share() {
    let dir = TempDir::new("summarize_test_share");
    dir.write(
        "src/lib.rs",
        "pub fn a() {}\npub fn b() {}\n\n#[cfg(test)]\nmod tests {\n    fn check_a() {}\n    fn check_b() {}\n}\n",
    );
    let out = overview(&dir);

    // The module and the two functions inside it are test code: three of five definitions.
    assert!(out.contains("5 defs (60% test)"), "{out}");
}
