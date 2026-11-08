//! The trait that bounds a generic parameter: `T: Putter`, `impl<T: Putter>` and `where T: Putter`.

use tree_sitter::Node;

use crate::node_text;

use super::types::trait_name;

/// The trait that bounds the generic parameter `param_name`, looked up on `function` and on the
/// `impl` blocks around it.
pub(super) fn generic_trait_bound(
    function: Node<'_>,
    param_name: &str,
    source: &str,
) -> Option<String> {
    std::iter::successors(Some(function), |node| node.parent())
        .filter(|node| matches!(node.kind(), "function_item" | "impl_item"))
        .find_map(|item| bound_on_item(item, param_name, source))
}

/// The bound of `param_name` in the type parameter list or the `where` clause of one item.
fn bound_on_item(item: Node<'_>, param_name: &str, source: &str) -> Option<String> {
    let mut cursor = item.walk();
    item.named_children(&mut cursor)
        .find_map(|child| match child.kind() {
            "type_parameters" => bound_in_list(
                child,
                &["type_parameter", "constrained_type_parameter"],
                &["name", "left"],
                param_name,
                source,
            ),
            "where_clause" => bound_in_list(
                child,
                &["where_predicate"],
                &["left", "type"],
                param_name,
                source,
            ),
            _ => None,
        })
}

/// The bound of `param_name` in the entries of `list` of the given kinds. `subject_fields` name
/// the field of an entry that holds the parameter it bounds.
fn bound_in_list(
    list: Node<'_>,
    entry_kinds: &[&str],
    subject_fields: &[&str],
    param_name: &str,
    source: &str,
) -> Option<String> {
    let mut cursor = list.walk();
    list.named_children(&mut cursor)
        .filter(|entry| entry_kinds.contains(&entry.kind()))
        .find_map(|entry| entry_bound(entry, subject_fields, param_name, source))
}

/// The trait of one `T: Trait` entry, if it bounds `param_name`. The subject is the first named
/// child when no field holds it, and the bounds are the named children after it.
fn entry_bound(
    entry: Node<'_>,
    subject_fields: &[&str],
    param_name: &str,
    source: &str,
) -> Option<String> {
    let subject = subject_fields
        .iter()
        .find_map(|field| entry.child_by_field_name(*field))
        .or_else(|| entry.named_child(0))?;
    if node_text(subject, source) != param_name {
        return None;
    }
    if let Some(bound) = entry
        .child_by_field_name("bounds")
        .and_then(|bounds| trait_name(bounds, source))
    {
        return Some(bound);
    }
    let mut cursor = entry.walk();
    entry
        .named_children(&mut cursor)
        .skip(1)
        .find_map(|child| trait_name(child, source))
}
