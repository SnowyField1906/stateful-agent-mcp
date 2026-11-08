//! Python grammar: parsing, docstrings and the folded view.

use tree_sitter::Node;

use crate::language::{Grammar, Ident, node_text};

use super::PythonSyntax;

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

    fn identifier(&self, node: Node<'_>) -> Option<Ident> {
        (node.kind() == "identifier").then_some(Ident::Value)
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
