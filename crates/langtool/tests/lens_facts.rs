//! The facts of a file come from one parse and answer what the older tools answer separately,
//! and the store keeps them until the file changes.

use std::sync::Arc;

use stateful_agent_langtool::engine::lens::facts::{Def, FileFacts, facts};
use stateful_agent_langtool::engine::lens::store::{FactStore, survey};
use stateful_agent_langtool::engine::references::Role;
use stateful_agent_langtool::engine::scan::ScanTarget;
use stateful_agent_language::registry;
use stateful_agent_testkit::TempDir;

const SOURCE: &str = "\
//! Sample module.

use std::fmt::Result as FmtResult;

/// A store of items.
pub struct Store {
    items: Vec<Item>,
}

impl Store {
    pub fn put(&self) {
        helper();
        self.log();
    }

    fn log(&self) {}
}

fn helper() -> FmtResult {
    Ok(())
}
";

fn parsed() -> FileFacts {
    facts(registry::for_path("lib.rs").unwrap(), SOURCE).unwrap()
}

fn def<'a>(file: &'a FileFacts, name: &str) -> &'a Def {
    file.defs
        .iter()
        .find(|def| def.symbol.name == name)
        .unwrap_or_else(|| panic!("no definition `{name}`"))
}

fn used(def: &Def) -> Vec<(&str, Role)> {
    def.uses
        .iter()
        .map(|used| (used.name.as_str(), used.role))
        .collect()
}

#[test]
fn a_file_gives_its_documentation_its_definitions_and_its_identifiers() {
    let file = parsed();
    assert_eq!(file.doc.as_deref(), Some("Sample module."));
    assert_eq!(file.lines, 21);
    let defs: Vec<(&str, &str)> = file
        .defs
        .iter()
        .map(|def| (def.symbol.kind.name, def.symbol.name.as_str()))
        .collect();
    assert_eq!(
        defs,
        [
            ("struct", "Store"),
            ("field", "items"),
            ("impl", "Store"),
            ("fn", "put"),
            ("fn", "log"),
            ("fn", "helper"),
        ]
    );
    for name in ["Store", "Item", "helper", "FmtResult"] {
        assert!(file.idents.contains(name), "`{name}` is not an identifier");
    }
    assert!(
        !file.idents.contains("Sample"),
        "a comment holds an identifier"
    );
}

#[test]
fn a_definition_knows_its_lines_its_parent_and_whether_it_is_exported() {
    let file = parsed();
    let put = def(&file, "put");
    assert_eq!(
        (put.symbol.first, put.symbol.head, put.symbol.last),
        (11, 11, 14)
    );
    assert_eq!(put.symbol.signature, "pub fn put(&self)");
    let parent = put.parent.map(|index| &file.defs[index]).unwrap();
    assert_eq!(parent.symbol.kind.name, "impl");

    let store = def(&file, "Store");
    assert_eq!(
        (store.symbol.first, store.symbol.head, store.symbol.last),
        (5, 6, 8)
    );
    assert_eq!(store.symbol.doc, "A store of items.");
    assert!(store.parent.is_none());

    let exported: Vec<(&str, bool)> = file
        .defs
        .iter()
        .map(|def| (def.symbol.name.as_str(), def.exported))
        .collect();
    assert_eq!(
        exported,
        [
            ("Store", true),
            ("items", false),
            ("Store", false),
            ("put", true),
            ("log", false),
            ("helper", false),
        ]
    );
}

#[test]
fn the_names_of_a_field_belong_to_its_struct_and_an_alias_gives_back_the_original() {
    let file = parsed();
    let store = used(def(&file, "Store"));
    assert!(store.contains(&("Item", Role::Type)), "{store:?}");
    assert!(store.contains(&("Vec", Role::Type)), "{store:?}");
    assert!(
        !store.iter().any(|(name, _)| *name == "items"),
        "a field name is a use: {store:?}"
    );

    let helper = def(&file, "helper");
    let result = helper
        .uses
        .iter()
        .find(|used| used.name == "Result")
        .unwrap_or_else(|| panic!("{:?}", used(helper)));
    assert_eq!(result.alias.as_deref(), Some("FmtResult"));
    assert_eq!(result.role, Role::Type);
}

#[test]
fn a_function_has_its_calls_and_the_names_it_uses() {
    let file = parsed();
    let put = def(&file, "put");
    let calls: Vec<(&str, Vec<usize>)> = put
        .calls
        .iter()
        .map(|call| (call.site.name.as_str(), call.lines.clone()))
        .collect();
    assert_eq!(calls, [("helper", vec![12]), ("log", vec![13])]);

    let names = used(put);
    assert!(names.contains(&("helper", Role::Call)), "{names:?}");
    assert!(names.contains(&("log", Role::Call)), "{names:?}");
    let helper = put.uses.iter().find(|used| used.name == "helper").unwrap();
    assert!(helper.site.is_some() && helper.lines == [12]);
    // The impl block holds the type it is written for, not what its methods use.
    let imp = file
        .defs
        .iter()
        .find(|def| def.symbol.kind.name == "impl")
        .unwrap();
    assert!(!used(imp).iter().any(|(name, _)| *name == "helper"));
}

#[test]
fn the_store_reads_a_file_again_only_when_it_changed() {
    let dir = TempDir::new("facts-store");
    dir.write("a.rs", "fn a() {}\n");
    let path = dir.root.join("a.rs");
    let store = FactStore::new(8);

    let first = store.facts(&path, "a.rs").unwrap();
    let again = store.facts(&path, "a.rs").unwrap();
    assert!(
        Arc::ptr_eq(&first, &again),
        "an unchanged file was parsed again"
    );

    dir.write("a.rs", "fn a() {}\nfn b() {}\n");
    let changed = store.facts(&path, "a.rs").unwrap();
    assert_eq!(changed.defs.len(), 2);
    assert_eq!(store.len(), 1);

    assert!(
        store.facts(&path, "notes.txt").is_none(),
        "a text file has facts"
    );
}

#[test]
fn the_store_is_capped_and_a_survey_reads_every_supported_file() {
    let dir = TempDir::new("facts-survey");
    dir.write("src/a.rs", "fn a() {}\n");
    dir.write("src/b.rs", "fn b() {}\n");
    dir.write("src/notes.txt", "not code\n");

    let store = FactStore::new(1);
    let root = dir.path_str();
    let target = ScanTarget {
        path: Some(&root),
        max_depth: 8,
        show_gitignored: None,
    };
    let found = survey(None, &target, &store).unwrap();
    assert_eq!(found.files.len(), 2);
    assert_eq!(found.skipped, 0);
    assert_eq!(store.len(), 1, "the store holds more than its capacity");
}
