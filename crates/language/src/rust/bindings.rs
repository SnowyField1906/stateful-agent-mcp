//! Which declaration of a name reaches a method call: the patterns that shadow it, the `let`
//! declarations before the call and the parameters of the function.

use tree_sitter::Node;

use crate::node_text;

use super::receiver::Receiver;
use super::types::{constructor_type, type_name};

/// Whether a pattern around `field` binds `name` and so shadows the declaration a type would come
/// from: the pattern of a `match` arm or a `for` loop it is in, or the `let` of the `if let` or
/// `while let` whose body it is in.
pub(super) fn shadowed_by_pattern(
    field: Node<'_>,
    function: Node<'_>,
    name: &str,
    source: &str,
) -> bool {
    std::iter::successors(Some(field), |node| node.parent())
        .take_while(|node| node.id() != function.id())
        .any(|scope| scope_binds(scope, field, name, source))
}

/// Whether `scope` is a construct whose pattern binds `name` for the node `inside` it.
fn scope_binds(scope: Node<'_>, inside: Node<'_>, name: &str, source: &str) -> bool {
    match scope.kind() {
        "match_arm" | "for_expression" => scope
            .child_by_field_name("pattern")
            .is_some_and(|pattern| pattern_binds(pattern, name, source)),
        "if_expression" => let_binds_in_body(scope, "consequence", inside, name, source),
        "while_expression" => let_binds_in_body(scope, "body", inside, name, source),
        _ => false,
    }
}

/// Whether the condition of an `if` or `while` has a `let` that binds `name`, and `inside` is in
/// the body that `let` is visible in.
fn let_binds_in_body(
    scope: Node<'_>,
    body_field: &str,
    inside: Node<'_>,
    name: &str,
    source: &str,
) -> bool {
    scope
        .child_by_field_name(body_field)
        .is_some_and(|body| contains_node(body, inside))
        && scope
            .child_by_field_name("condition")
            .is_some_and(|condition| contains_let_binding(condition, name, source))
}

fn contains_let_binding(node: Node<'_>, name: &str, source: &str) -> bool {
    if node.kind() == "let_condition"
        && node
            .child_by_field_name("pattern")
            .is_some_and(|pattern| pattern_binds(pattern, name, source))
    {
        return true;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| contains_let_binding(child, name, source))
}

/// Whether `descendant` is `ancestor` or lies below it.
fn contains_node(ancestor: Node<'_>, descendant: Node<'_>) -> bool {
    std::iter::successors(Some(descendant), |node| node.parent())
        .any(|node| node.id() == ancestor.id())
}

/// The nearest block that holds `node`, or `node` itself if it is one.
pub(super) fn nearest_block(node: Node<'_>) -> Option<Node<'_>> {
    std::iter::successors(Some(node), |node| node.parent()).find(|node| node.kind() == "block")
}

/// The number of blocks around `block`.
fn block_depth(block: Node<'_>) -> usize {
    std::iter::successors(block.parent(), |node| node.parent())
        .filter(|node| node.kind() == "block")
        .count()
}

/// A `let` that binds the name: where it starts, how deeply its block is nested and the type it
/// gives the name, if one is known.
struct Binding {
    start: usize,
    depth: usize,
    ty: Option<Receiver>,
}

/// What a search for the `let` declarations of a name in one function holds fixed.
struct Lookup<'a> {
    function: Node<'a>,
    call_block: Node<'a>,
    name: &'a str,
    before: usize,
    source: &'a str,
}

/// The type of the `let` binding of `name` that reaches a call in `call_block` at byte `before`:
/// the one in the most deeply nested block that holds the call, and the latest of those. `None`
/// when no `let` binds the name; `Some(None)` when the binding has no known type.
pub(super) fn local_binding_type(
    function: Node<'_>,
    call_block: Node<'_>,
    name: &str,
    before: usize,
    source: &str,
) -> Option<Option<Receiver>> {
    let lookup = Lookup {
        function,
        call_block,
        name,
        before,
        source,
    };
    let mut bindings = Vec::new();
    lookup.collect(function, &mut bindings);
    bindings
        .into_iter()
        .max_by_key(|binding| (binding.depth, binding.start))
        .map(|binding| binding.ty)
}

impl Lookup<'_> {
    /// Collect the bindings below `node` that start before the call, leaving nested functions out.
    fn collect(&self, node: Node<'_>, bindings: &mut Vec<Binding>) {
        if node.start_byte() >= self.before {
            return;
        }
        if node.id() != self.function.id() && node.kind() == "function_item" {
            return;
        }
        bindings.extend(self.binding_of(node));

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect(child, bindings);
        }
    }

    /// The binding `node` declares, if it is a `let` of the name in a block that holds the call.
    fn binding_of(&self, node: Node<'_>) -> Option<Binding> {
        if node.kind() != "let_declaration" {
            return None;
        }
        let pattern = node.child_by_field_name("pattern")?;
        if !pattern_binds(pattern, self.name, self.source) {
            return None;
        }
        let block = nearest_block(node)?;
        if !contains_node(block, self.call_block) {
            return None;
        }
        let ty = node
            .child_by_field_name("type")
            .and_then(|ty| type_name(ty, self.source))
            .or_else(|| {
                node.child_by_field_name("value")
                    .and_then(|value| constructor_type(value, self.function, self.source))
            });
        Some(Binding {
            start: node.start_byte(),
            depth: block_depth(block),
            ty,
        })
    }
}

/// Whether a pattern binds `name`.
fn pattern_binds(pattern: Node<'_>, name: &str, source: &str) -> bool {
    match pattern.kind() {
        "type_identifier" | "field_identifier" | "scoped_identifier" => return false,
        "identifier" | "shorthand_field_identifier" => return node_text(pattern, source) == name,
        _ => {}
    }
    // The guard of a match arm (`None if root.starts_with(..)`) is a child of its `match_pattern`
    // but only uses names; it binds none.
    let guard = if pattern.kind() == "match_pattern" {
        pattern.child_by_field_name("condition")
    } else {
        None
    };
    let mut cursor = pattern.walk();
    pattern
        .named_children(&mut cursor)
        .filter(|child| guard.is_none_or(|guard| guard.id() != child.id()))
        .any(|child| pattern_binds(child, name, source))
}

/// The name a parameter pattern binds: `store` for `store`, `mut store` and `ref store`.
fn binding_name(pattern: Node<'_>, source: &str) -> Option<String> {
    match pattern.kind() {
        "identifier" => Some(node_text(pattern, source).to_string()),
        "mut_pattern" | "ref_pattern" => pattern
            .child_by_field_name("pattern")
            .or_else(|| pattern.named_child(0))
            .and_then(|inner| binding_name(inner, source)),
        _ => None,
    }
}

/// The type a parameter of `function` named `name` is declared with.
pub(super) fn parameter_type(function: Node<'_>, name: &str, source: &str) -> Option<Receiver> {
    let parameters = function.child_by_field_name("parameters")?;
    let mut cursor = parameters.walk();
    let parameter = parameters
        .named_children(&mut cursor)
        .filter(|parameter| parameter.kind() == "parameter")
        .find(|parameter| {
            parameter
                .child_by_field_name("pattern")
                .and_then(|pattern| binding_name(pattern, source))
                .as_deref()
                == Some(name)
        })?;
    type_name(parameter.child_by_field_name("type")?, source)
}
