//! Python grammar: parsing, docstrings and the folded view.

use tree_sitter::Node;

use crate::grammar::is_field;
use crate::{Grammar, Ident, node_text};

use super::PythonSyntax;

/// Pattern nodes that bind the names written inside them.
const PATTERNS: &[&str] = &[
    "pattern_list",
    "tuple_pattern",
    "list_pattern",
    "list_splat_pattern",
    "dictionary_splat_pattern",
];

/// Nodes and the field that binds the names written in it.
const BINDERS: &[(&str, &str)] = &[
    ("for_statement", "left"),
    ("for_in_clause", "left"),
    ("assignment", "left"),
    ("augmented_assignment", "left"),
    ("named_expression", "name"),
    ("default_parameter", "name"),
    ("typed_default_parameter", "name"),
];

impl Grammar for PythonSyntax {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }

    fn block_kinds(&self) -> &'static [&'static str] {
        &["block"]
    }

    fn elision(&self) -> &'static str {
        "..."
    }

    fn is_container(&self, node: Node<'_>) -> bool {
        node.kind() == "block"
            && node
                .parent()
                .is_some_and(|p| p.kind() == "class_definition")
    }

    fn is_doc(&self, node: Node<'_>, _source: &str) -> bool {
        node.kind() == "comment"
    }

    fn is_attribute(&self, node: Node<'_>) -> bool {
        node.kind() == "decorator"
    }

    fn unit_node<'a>(&self, node: Node<'a>) -> Node<'a> {
        match node.parent() {
            Some(parent) if parent.kind() == "decorated_definition" => parent,
            _ => node,
        }
    }

    fn doc_line(&self, node: Node<'_>, source: &str) -> Option<String> {
        first_docstring_line(node.child_by_field_name("body")?, source)
    }

    fn body_summary(&self, block: Node<'_>, source: &str) -> Option<String> {
        first_docstring_line(block, source).map(|line| format!("\"\"\"{line}\"\"\""))
    }

    /// The first line of the docstring of the module.
    fn file_doc(&self, root: Node<'_>, source: &str) -> Option<String> {
        first_docstring_line(root, source)
    }

    fn identifier(&self, node: Node<'_>) -> Option<Ident> {
        (node.kind() == "identifier").then_some(Ident::Value)
    }

    fn is_binding(&self, node: Node<'_>) -> bool {
        if node.kind() != "identifier" {
            return false;
        }
        let mut inner = node;
        while let Some(parent) = inner.parent() {
            let kind = parent.kind();
            if matches!(kind, "parameters" | "lambda_parameters" | "typed_parameter") {
                return true;
            }
            if let Some(&(_, field)) = BINDERS.iter().find(|(binder, _)| *binder == kind) {
                return is_field(parent, field, inner);
            }
            if !PATTERNS.contains(&kind) {
                return false;
            }
            inner = parent;
        }
        false
    }

    fn text_kinds(&self) -> &'static [&'static str] {
        &["comment", "string"]
    }
}

/// The first line of the docstring that starts `block`, if its first statement is a string.
fn first_docstring_line(block: Node<'_>, source: &str) -> Option<String> {
    let mut cursor = block.walk();
    let stmt = block
        .named_children(&mut cursor)
        .find(|n| n.kind() != "comment")?;
    if stmt.kind() != "expression_statement" {
        return None;
    }
    let literal = stmt.named_child(0).filter(|n| n.kind() == "string")?;
    let text = node_text(literal, source).trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let body = ["\"\"\"", "'''", "\"", "'"]
        .iter()
        .find_map(|q| text.strip_prefix(*q))?;
    let line = body.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_end_matches(['"', '\'']).trim();
    (!line.is_empty()).then(|| line.to_string())
}
