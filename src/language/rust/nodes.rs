//! Helpers over Rust paths that more than one concern needs.

use tree_sitter::Node;

use crate::language::node_text;

/// The last segment of a path: `b` for `a::b` and for `b`.
pub(super) fn last_segment(node: Node<'_>, source: &str) -> String {
    let name = match node.kind() {
        "scoped_identifier" => node.child_by_field_name("name").unwrap_or(node),
        _ => node,
    };
    node_text(name, source).to_string()
}

/// The last segment of the path in front of a call's name; the type of a generic type.
pub(super) fn path_segment(node: Node<'_>, source: &str) -> String {
    let node = match node.kind() {
        "generic_type" => node.child_by_field_name("type").unwrap_or(node),
        _ => node,
    };
    last_segment(node, source)
}

/// The name a path to a type ends in: `Store` for `Store`, `store::Store` and `Store<T>`.
pub(super) fn type_path_name(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(node_text(node, source).to_string()),
        "scoped_type_identifier" => {
            Some(node_text(node.child_by_field_name("name")?, source).to_string())
        }
        "generic_type" => type_path_name(node.child_by_field_name("type")?, source),
        _ => None,
    }
}
