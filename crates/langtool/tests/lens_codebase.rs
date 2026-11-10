//! Support code of tests is judged within one project: a name that another project of the scan
//! shares is not a use of the file.

use std::sync::Arc;

use stateful_agent_langtool::engine::lens::codebase::Codebase;
use stateful_agent_langtool::engine::lens::facts::{FileFacts, facts};
use stateful_agent_langtool::engine::origins::Origins;
use stateful_agent_language::registry;
use stateful_agent_testkit::TempDir;

/// A fixture builder that only tests of its own project use.
const KIT: &str = "pub struct Fixture {\n    pub id: u32,\n}\n";

/// A test of project `a` that builds on the fixture.
const TEST: &str = "pub fn builds(fixture: Fixture) {}\n";

/// Code that is no test and mentions the fixture by name.
const CODE: &str = "pub fn run(fixture: Fixture) {}\n";

/// The codebase of `sources` (label and text), which lie below a scratch folder with two projects,
/// `a` and `b`, each with its own `Cargo.toml`. The files named in `tests` are test code, as the
/// store makes a file of a test folder.
fn codebase(sources: &[(&str, &str)], tests: &[&str]) -> Codebase {
    let dir = TempDir::new("lens_codebase");
    for project in ["a", "b"] {
        dir.write(
            &format!("{project}/Cargo.toml"),
            &format!("[package]\nname = \"{project}\"\nversion = \"0.1.0\"\n"),
        );
    }
    let files: Vec<(String, Arc<FileFacts>)> = sources
        .iter()
        .map(|(label, source)| {
            let mut parsed = facts(registry::for_path(label).unwrap(), source).unwrap();
            if tests.contains(label) {
                for def in &mut parsed.defs {
                    def.symbol.test = true;
                }
            }
            (label.to_string(), Arc::new(parsed))
        })
        .collect();
    Codebase::new(files, &Origins::new(&dir.root, &[]))
}

/// Whether the file called `label` holds test code only.
fn is_test_file(codebase: &Codebase, label: &str) -> bool {
    let file = codebase
        .files
        .iter()
        .position(|(name, _)| name == label)
        .unwrap_or_else(|| panic!("no file {label}"));
    codebase.is_test_file(file)
}

#[test]
fn a_file_that_only_tests_of_its_project_mention_is_support_code() {
    let codebase = codebase(
        &[("a/src/kit.rs", KIT), ("a/tests/uses.rs", TEST)],
        &["a/tests/uses.rs"],
    );
    assert!(is_test_file(&codebase, "a/src/kit.rs"));
}

#[test]
fn code_of_the_same_project_that_mentions_a_file_keeps_it_from_being_support_code() {
    let codebase = codebase(
        &[
            ("a/src/kit.rs", KIT),
            ("a/src/app.rs", CODE),
            ("a/tests/uses.rs", TEST),
        ],
        &["a/tests/uses.rs"],
    );
    assert!(!is_test_file(&codebase, "a/src/kit.rs"));
}

#[test]
fn a_name_in_common_with_code_of_another_project_does_not_keep_a_file_from_being_support_code() {
    let codebase = codebase(
        &[
            ("a/src/kit.rs", KIT),
            ("a/tests/uses.rs", TEST),
            ("b/src/app.rs", CODE),
        ],
        &["a/tests/uses.rs"],
    );
    assert!(is_test_file(&codebase, "a/src/kit.rs"));
    assert!(!is_test_file(&codebase, "b/src/app.rs"));
}
