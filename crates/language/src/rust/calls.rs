//! Rust calls: the name a call ends in and how it was written.

use tree_sitter::Node;

use crate::{CallKind, CallSite, Calls, Declarations, node_text};

use super::RustSyntax;
use super::nodes::path_segment;
use super::receiver::{self, Receiver};

impl Calls for RustSyntax {
    fn types_inherit_methods(&self) -> bool {
        false
    }

    fn call_name(&self, node: Node<'_>, source: &str) -> Option<String> {
        if node.kind() != "call_expression" {
            return None;
        }
        target_name(node.child_by_field_name("function")?, source)
    }

    fn call_site(&self, node: Node<'_>, source: &str) -> Option<CallSite> {
        if node.kind() != "call_expression" {
            return None;
        }
        let site = target_site(node.child_by_field_name("function")?, source)?;
        // `Some(x)`, `Err(e)` and `Enum::Variant(x)` construct values; functions are snake_case.
        if self.is_type_name(&site.name) {
            return None;
        }
        Some(site)
    }
}

/// The name a call target ends in and how the call was written.
fn target_site(node: Node<'_>, source: &str) -> Option<CallSite> {
    let site = |name: Node<'_>, kind: CallKind, root: Option<String>| CallSite {
        name: node_text(name, source).to_string(),
        kind,
        root,
    };
    match node.kind() {
        "identifier" => Some(site(node, CallKind::Free, None)),
        "scoped_identifier" => {
            let name = node.child_by_field_name("name")?;
            let path = node.child_by_field_name("path");
            let kind = match path {
                Some(path) => match path_segment(path, source).as_str() {
                    "Self" => CallKind::Own,
                    "self" | "crate" | "super" => CallKind::Free,
                    other => CallKind::Path(other.to_string()),
                },
                None => CallKind::Free,
            };
            let root = path.and_then(|path| first_segment(path, source));
            Some(site(name, kind, root))
        }
        "field_expression" => {
            let name = node.child_by_field_name("field")?;
            let receiver = node.child_by_field_name("value")?;
            let kind = if node_text(receiver, source) == "self" {
                CallKind::Own
            } else {
                match receiver::infer_type(node, source) {
                    Some(Receiver::Type(ty)) => CallKind::MethodOf(ty),
                    Some(Receiver::Trait(name)) => CallKind::Dyn(name),
                    None => CallKind::Method,
                }
            };
            Some(site(name, kind, None))
        }
        "generic_function" => target_site(node.child_by_field_name("function")?, source),
        _ => None,
    }
}

/// The first segment of a path: `std` for `std::fs::read`, `Vec` for `Vec::<u8>::new`.
fn first_segment(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" | "crate" | "self" | "super" => {
            Some(node_text(node, source).to_string())
        }
        "scoped_identifier" | "scoped_type_identifier" => {
            first_segment(node.child_by_field_name("path")?, source)
        }
        "generic_type" => first_segment(node.child_by_field_name("type")?, source),
        _ => None,
    }
}

/// The plain name a call target ends in: `foo` for `foo`, `a::foo`, `foo::<T>` and `x.foo`.
fn target_name(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" => Some(node_text(node, source).to_string()),
        "scoped_identifier" => target_name(node.child_by_field_name("name")?, source),
        "field_expression" => target_name(node.child_by_field_name("field")?, source),
        "generic_function" => target_name(node.child_by_field_name("function")?, source),
        _ => None,
    }
}
