//! Conservative local receiver-type inference for Rust method calls.
//!
//! This is syntax-based, not compiler type checking. It resolves named types from annotations
//! and local constructors; bindings introduced by patterns without a known type stay unknown.
//!
//! `bindings` finds the declaration of the name that reaches the call, `types` turns type
//! expressions and constructors into receivers, and `bounds` resolves a generic parameter to the
//! trait that bounds it.

use tree_sitter::Node;

use crate::node_text;

use super::bindings::{local_binding_type, nearest_block, parameter_type, shadowed_by_pattern};
use super::bounds::generic_trait_bound;
use super::types::self_type;

/// What is known about the type of the receiver of a method call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Receiver {
    /// A named type: `Store` for `Store`, `&Store` and `Box<Store>`.
    Type(String),
    /// A trait object or `impl Trait`: `Syntax` for `&dyn Syntax`, `Box<dyn Syntax>` and
    /// `impl Syntax`.
    Trait(String),
}

/// Infer the type of the receiver in a `field_expression` such as `value.method`.
pub(super) fn infer_type(field: Node<'_>, source: &str) -> Option<Receiver> {
    let receiver = field.child_by_field_name("value")?;
    if receiver.kind() != "identifier" {
        return None;
    }
    let name = node_text(receiver, source);
    let function = enclosing_function(field)?;
    if shadowed_by_pattern(field, function, name, source) {
        return None;
    }
    let call_block = nearest_block(field)?;

    if let Some(local) = local_binding_type(function, call_block, name, field.start_byte(), source)
    {
        return local.and_then(|ty| resolve_self_type(ty, function, source));
    }
    parameter_type(function, name, source).and_then(|ty| resolve_self_type(ty, function, source))
}

/// The function the node is in, unless a closure lies in between: its parameters and bindings are
/// not looked up.
fn enclosing_function(node: Node<'_>) -> Option<Node<'_>> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "closure_expression" => return None,
            "function_item" => return Some(parent),
            _ => current = parent,
        }
    }
    None
}

/// Resolve what a type expression leaves open: `Self` is the type of the enclosing `impl`, and a
/// name that a generic parameter declares is the trait that bounds it.
fn resolve_self_type(ty: Receiver, function: Node<'_>, source: &str) -> Option<Receiver> {
    match ty {
        Receiver::Type(name) if name == "Self" => self_type(function, source),
        Receiver::Type(name) => Some(match generic_trait_bound(function, &name, source) {
            Some(bound) => Receiver::Trait(bound),
            None => Receiver::Type(name),
        }),
        other => Some(other),
    }
}
