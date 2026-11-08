//! What Python defines.

use std::path::Path;

use tree_sitter::Node;

use crate::language::{Declarations, Definition, Kind, is_conventional_test_path, node_text};

use super::PythonSyntax;

static DEF: Kind = Kind::new("def")
    .aka(&["function", "func", "method"])
    .function();
static CLASS: Kind = Kind::new("class").declares_type().owner();
static VAR: Kind = Kind::new("var").aka(&["variable"]);
static FIELD: Kind = Kind::new("field").on_request();

static KINDS: &[&Kind] = &[&DEF, &CLASS, &VAR, &FIELD];

impl Declarations for PythonSyntax {
    fn kinds(&self) -> &'static [&'static Kind] {
        KINDS
    }

    fn definition(&self, node: Node<'_>, source: &str) -> Option<Definition> {
        match node.kind() {
            "function_definition" => named(&DEF, node, source),
            "class_definition" => named(&CLASS, node, source).map(|class| {
                let tests = class.name.starts_with("Test");
                class.holding_tests(tests)
            }),
            "assignment" => {
                let kind: &'static Kind = if is_module_level(node) {
                    &VAR
                } else if is_class_level(node) {
                    &FIELD
                } else {
                    return None;
                };
                let left = node.child_by_field_name("left")?;
                (left.kind() == "identifier")
                    .then(|| Definition::new(kind, node_text(left, source)))
            }
            _ => None,
        }
    }

    /// A definition names itself in `name`; an assignment names what it assigns in `left`.
    fn name_fields(&self) -> &'static [&'static str] {
        &["name", "left"]
    }

    fn is_test_path(&self, path: &str) -> bool {
        is_conventional_test_path(path)
            || Path::new(path).file_stem().and_then(|stem| stem.to_str()) == Some("conftest")
    }
}

/// The definition whose name is the `name` field of `node`.
fn named(kind: &'static Kind, node: Node<'_>, source: &str) -> Option<Definition> {
    let name = node.child_by_field_name("name")?;
    Some(Definition::new(kind, node_text(name, source)))
}

/// Whether an assignment is a statement directly in the module body.
fn is_module_level(node: Node<'_>) -> bool {
    node.parent()
        .filter(|p| p.kind() == "expression_statement")
        .and_then(|p| p.parent())
        .is_some_and(|g| g.kind() == "module")
}

/// Whether an assignment is a statement directly in a class body.
fn is_class_level(node: Node<'_>) -> bool {
    node.parent()
        .filter(|p| p.kind() == "expression_statement")
        .and_then(|p| p.parent())
        .filter(|b| b.kind() == "block")
        .and_then(|b| b.parent())
        .is_some_and(|c| c.kind() == "class_definition")
}
