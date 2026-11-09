//! What Python defines.

use std::path::Path;

use tree_sitter::Node;

use crate::{Declarations, Definition, Kind, is_conventional_test_path, node_text};

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

    /// A name without a leading underscore, in a class that is exported too and not inside a
    /// function. When the module lists `__all__`, that list decides for its top-level names.
    fn exported(&self, node: Node<'_>, source: &str) -> bool {
        let Some(definition) = self.definition(node, source) else {
            return true;
        };
        // The name of the definition, then the names of the classes around it, innermost first.
        let mut names = vec![definition.name];
        let mut module = None;
        let mut above = node.parent();
        while let Some(parent) = above {
            match parent.kind() {
                "function_definition" => return false,
                "class_definition" => {
                    if let Some(name) = parent.child_by_field_name("name") {
                        names.push(node_text(name, source).to_string());
                    }
                }
                "module" => module = Some(parent),
                _ => {}
            }
            above = parent.parent();
        }
        let listed = module.and_then(|module| all_names(module, source));
        match (listed, names.split_last()) {
            (Some(listed), Some((top, members))) => {
                listed.contains(top) && members.iter().all(|name| !name.starts_with('_'))
            }
            _ => names.iter().all(|name| !name.starts_with('_')),
        }
    }
}

/// The names that the module lists in `__all__`, when it has one.
fn all_names(module: Node<'_>, source: &str) -> Option<Vec<String>> {
    let mut names: Option<Vec<String>> = None;
    let mut cursor = module.walk();
    for statement in module.named_children(&mut cursor) {
        let Some(assignment) = statement
            .named_child(0)
            .filter(|_| statement.kind() == "expression_statement")
            .filter(|node| matches!(node.kind(), "assignment" | "augmented_assignment"))
        else {
            continue;
        };
        let is_all = assignment
            .child_by_field_name("left")
            .is_some_and(|left| node_text(left, source) == "__all__");
        let Some(list) = assignment.child_by_field_name("right").filter(|_| is_all) else {
            continue;
        };
        let mut inner = list.walk();
        names.get_or_insert_with(Vec::new).extend(
            list.named_children(&mut inner)
                .filter(|item| item.kind() == "string")
                .map(|item| string_value(node_text(item, source))),
        );
    }
    names
}

/// The text of a string literal without its prefix letters and quotes.
fn string_value(literal: &str) -> String {
    literal
        .trim_start_matches(|c: char| c.is_ascii_alphabetic())
        .trim_matches(['"', '\''])
        .to_string()
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
