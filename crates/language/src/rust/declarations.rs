//! What Rust defines.

use tree_sitter::Node;

use crate::{Declarations, Definition, Kind, node_text};

use super::RustSyntax;
use super::nodes::type_path_name;

static FN: Kind = Kind::new("fn")
    .aka(&["function", "func", "method"])
    .function();
static STRUCT: Kind = Kind::new("struct").declares_type();
static ENUM: Kind = Kind::new("enum").declares_type();
static UNION: Kind = Kind::new("union").declares_type();
static TRAIT: Kind = Kind::new("trait")
    .aka(&["interface"])
    .declares_type()
    .owner()
    .interface();
static TYPE: Kind = Kind::new("type").declares_type();
static CONST: Kind = Kind::new("const").aka(&["constant"]);
static STATIC: Kind = Kind::new("static");
static MOD: Kind = Kind::new("mod").aka(&["module"]).module();
static MACRO: Kind = Kind::new("macro");
static IMPL: Kind = Kind::new("impl").owner();
static VARIANT: Kind = Kind::new("variant").on_request();
static FIELD: Kind = Kind::new("field").on_request();

static KINDS: &[&Kind] = &[
    &FN, &STRUCT, &ENUM, &UNION, &TRAIT, &TYPE, &CONST, &STATIC, &MOD, &MACRO, &IMPL, &VARIANT,
    &FIELD,
];

impl Declarations for RustSyntax {
    fn kinds(&self) -> &'static [&'static Kind] {
        KINDS
    }

    fn definition(&self, node: Node<'_>, source: &str) -> Option<Definition> {
        let kind: &'static Kind = match node.kind() {
            "function_item" | "function_signature_item" => &FN,
            "struct_item" => &STRUCT,
            "enum_item" => &ENUM,
            "enum_variant" => &VARIANT,
            "field_declaration" => &FIELD,
            "union_item" => &UNION,
            "trait_item" => &TRAIT,
            "type_item" => &TYPE,
            "const_item" => &CONST,
            "static_item" => &STATIC,
            "mod_item" => &MOD,
            "macro_definition" => &MACRO,
            "impl_item" => {
                let target = node.child_by_field_name("type")?;
                let trait_name = node
                    .child_by_field_name("trait")
                    .and_then(|node| type_path_name(node, source));
                return Some(
                    Definition::new(&IMPL, node_text(target, source)).implementing(trait_name),
                );
            }
            _ => return None,
        };
        let name = node_text(node.child_by_field_name("name")?, source);
        let tests = kind.module && matches!(name, "tests" | "test");
        Some(Definition::new(kind, name).holding_tests(tests))
    }

    /// `pub` and nothing narrower. A method of a trait is as public as the trait, and a member
    /// of a trait implementation is reached through the trait, never on its own.
    fn exported(&self, node: Node<'_>, source: &str) -> bool {
        match node.kind() {
            "impl_item" => false,
            "enum_variant" => {
                ancestor(node, "enum_item").is_some_and(|owner| self.exported(owner, source))
            }
            "macro_definition" => has_macro_export(node, source),
            _ => match node
                .parent()
                .filter(|parent| parent.kind() == "declaration_list")
                .and_then(|parent| parent.parent())
            {
                Some(owner) if owner.kind() == "trait_item" => self.exported(owner, source),
                Some(owner)
                    if owner.kind() == "impl_item"
                        && owner.child_by_field_name("trait").is_some() =>
                {
                    false
                }
                _ => is_pub(node, source),
            },
        }
    }
}

/// The nearest ancestor of `node` of the node kind `kind`.
fn ancestor<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    std::iter::successors(node.parent(), |node| node.parent()).find(|node| node.kind() == kind)
}

/// Whether the item `node` is written with a plain `pub`; `pub(crate)` and `pub(super)` are not.
fn is_pub(node: Node<'_>, source: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == "visibility_modifier" && node_text(child, source) == "pub")
}

/// Whether a `macro_rules!` carries `#[macro_export]`, which puts it at the root of its crate.
fn has_macro_export(node: Node<'_>, source: &str) -> bool {
    let mut above = node.prev_sibling();
    while let Some(sibling) = above {
        match sibling.kind() {
            "attribute_item" if node_text(sibling, source).contains("macro_export") => return true,
            "attribute_item" | "line_comment" | "block_comment" => {}
            _ => break,
        }
        above = sibling.prev_sibling();
    }
    false
}
