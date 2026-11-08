//! The receiver types that type expressions, constructor expressions and `impl` blocks name.

use tree_sitter::Node;

use crate::node_text;

use super::nodes::{path_segment, type_path_name};
use super::receiver::Receiver;

/// The receiver type a type expression names: the named type behind references and the smart
/// pointers `Box`, `Rc` and `Arc`, or the trait of `dyn Trait` and `impl Trait`.
pub(super) fn type_name(node: Node<'_>, source: &str) -> Option<Receiver> {
    match node.kind() {
        "type_identifier" | "scoped_type_identifier" => {
            type_path_name(node, source).map(Receiver::Type)
        }
        "generic_type" => {
            let name = type_path_name(node, source)?;
            if is_smart_pointer(&name) {
                first_type(node.child_by_field_name("type_arguments")?, source)
            } else {
                Some(Receiver::Type(name))
            }
        }
        "reference_type" | "pointer_type" => type_name(node.child_by_field_name("type")?, source),
        // `str`, `bool`, `u8`: names the language reserves, so the standard library's.
        "primitive_type" => Some(Receiver::Type(node_text(node, source).to_string())),
        // `[T]` and `[T; N]` have no name of their own. A name no type can have keeps the
        // receiver typed, and the project's only methods on it are those of a trait it implements.
        "array_type" => Some(Receiver::Type("[]".to_string())),
        "dynamic_type" | "abstract_type" => {
            trait_name(node.child_by_field_name("trait")?, source).map(Receiver::Trait)
        }
        // `dyn Trait + Send`: the first bound that names a type.
        "bounded_type" => first_type(node, source),
        _ => None,
    }
}

/// The trait a bound names: the first of `A + B`, without generic arguments.
pub(super) fn trait_name(node: Node<'_>, source: &str) -> Option<String> {
    if matches!(node.kind(), "bounded_type" | "trait_bounds") {
        let mut cursor = node.walk();
        return node
            .named_children(&mut cursor)
            .find_map(|bound| trait_name(bound, source));
    }
    type_path_name(node, source)
}

/// The first child of `node` that names a type: a type argument of `Box<..>` or a bound of
/// `dyn A + Send`.
fn first_type(node: Node<'_>, source: &str) -> Option<Receiver> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(|child| type_name(child, source))
}

/// Pointers whose methods are those of the type they point to.
fn is_smart_pointer(name: &str) -> bool {
    matches!(name, "Box" | "Rc" | "Arc")
}

/// The receiver type of a value that a constructor makes: `Store::new()`, `Store(..)`,
/// `Store { .. }` and the unit struct `Store`. `function` is the function the value is in, for
/// `Self`.
pub(super) fn constructor_type(
    value: Node<'_>,
    function: Node<'_>,
    source: &str,
) -> Option<Receiver> {
    let name = match value.kind() {
        "call_expression" => {
            let mut callee = value.child_by_field_name("function")?;
            while callee.kind() == "generic_function" {
                callee = callee.child_by_field_name("function")?;
            }
            match callee.kind() {
                "identifier" | "type_identifier" => node_text(callee, source).to_string(),
                "scoped_identifier" => path_segment(callee.child_by_field_name("path")?, source),
                _ => return None,
            }
        }
        "struct_expression" => {
            let name = value.child_by_field_name("name")?;
            type_path_name(name, source).unwrap_or_else(|| path_segment(name, source))
        }
        "identifier" | "type_identifier" => node_text(value, source).to_string(),
        _ => return None,
    };
    named_type(name, function, source)
}

/// The type a name in a constructor stands for: the `impl` type for `Self`, any name that starts
/// like a type, and nothing for the lowercase names of modules and functions.
fn named_type(name: String, function: Node<'_>, source: &str) -> Option<Receiver> {
    if name == "Self" {
        return self_type(function, source);
    }
    starts_type_name(&name).then_some(Receiver::Type(name))
}

fn starts_type_name(name: &str) -> bool {
    name.starts_with(char::is_uppercase)
}

/// The type of the `impl` block that holds `function`.
pub(super) fn self_type(function: Node<'_>, source: &str) -> Option<Receiver> {
    std::iter::successors(function.parent(), |node| node.parent())
        .find(|node| node.kind() == "impl_item")?
        .child_by_field_name("type")
        .and_then(|ty| type_name(ty, source))
}
