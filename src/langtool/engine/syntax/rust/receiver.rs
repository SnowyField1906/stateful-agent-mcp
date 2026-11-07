//! Conservative local receiver-type inference for Rust method calls.
//!
//! This is syntax-based, not compiler type checking. It resolves named types from annotations
//! and local constructors; bindings introduced by patterns without a known type stay unknown.

use tree_sitter::Node;

use super::{node_text, path_segment};

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
    if has_untyped_pattern_binding(field, function, name, source) {
        return None;
    }
    let call_block = nearest_block(field)?;

    if let Some(local) = local_binding_type(function, call_block, name, field.start_byte(), source)
    {
        return local.and_then(|ty| resolve_self_type(ty, function, source));
    }
    parameter_type(function, name, source).and_then(|ty| resolve_self_type(ty, function, source))
}

fn has_untyped_pattern_binding(
    field: Node<'_>,
    function: Node<'_>,
    name: &str,
    source: &str,
) -> bool {
    let mut current = field;
    while current.id() != function.id() {
        if matches!(current.kind(), "match_arm" | "for_expression")
            && current
                .child_by_field_name("pattern")
                .is_some_and(|pattern| pattern_binds(pattern, name, source))
        {
            return true;
        }
        if matches!(current.kind(), "if_expression" | "while_expression") {
            let body = if current.kind() == "if_expression" {
                current.child_by_field_name("consequence")
            } else {
                current.child_by_field_name("body")
            };
            let condition = current.child_by_field_name("condition");
            if body.is_some_and(|body| contains_node(body, field))
                && condition.is_some_and(|condition| contains_let_binding(condition, name, source))
            {
                return true;
            }
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }
    false
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

fn contains_node(ancestor: Node<'_>, descendant: Node<'_>) -> bool {
    let mut current = Some(descendant);
    while let Some(node) = current {
        if node.id() == ancestor.id() {
            return true;
        }
        current = node.parent();
    }
    false
}

fn enclosing_function(node: Node<'_>) -> Option<Node<'_>> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.kind() == "closure_expression" {
            return None;
        }
        if parent.kind() == "function_item" {
            return Some(parent);
        }
        current = parent;
    }
    None
}

fn nearest_block(node: Node<'_>) -> Option<Node<'_>> {
    let mut current = node;
    loop {
        if current.kind() == "block" {
            return Some(current);
        }
        current = current.parent()?;
    }
}

fn local_binding_type(
    function: Node<'_>,
    call_block: Node<'_>,
    name: &str,
    before: usize,
    source: &str,
) -> Option<Option<Receiver>> {
    let mut bindings = Vec::new();
    collect_bindings(
        function,
        function,
        call_block,
        name,
        before,
        source,
        &mut bindings,
    );
    bindings
        .into_iter()
        .max_by_key(|(start, depth, _)| (*depth, *start))
        .map(|(_, _, ty)| ty)
}

fn collect_bindings(
    node: Node<'_>,
    function: Node<'_>,
    call_block: Node<'_>,
    name: &str,
    before: usize,
    source: &str,
    bindings: &mut Vec<(usize, usize, Option<Receiver>)>,
) {
    if node.start_byte() >= before {
        return;
    }
    if node.id() != function.id() && node.kind() == "function_item" {
        return;
    }

    if node.kind() == "let_declaration"
        && let Some(pattern) = node.child_by_field_name("pattern")
        && pattern_binds(pattern, name, source)
        && let Some(block) = nearest_block(node)
        && is_ancestor_block(block, call_block)
    {
        let ty = node
            .child_by_field_name("type")
            .and_then(|ty| type_name(ty, source))
            .or_else(|| {
                node.child_by_field_name("value")
                    .and_then(|value| constructor_type(value, function, source))
            });
        bindings.push((node.start_byte(), block_depth(block), ty));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_bindings(child, function, call_block, name, before, source, bindings);
    }
}

fn pattern_binds(pattern: Node<'_>, name: &str, source: &str) -> bool {
    match pattern.kind() {
        "type_identifier" | "field_identifier" | "scoped_identifier" => return false,
        "identifier" | "shorthand_field_identifier" => return node_text(pattern, source) == name,
        _ => {}
    }
    let mut cursor = pattern.walk();
    pattern
        .named_children(&mut cursor)
        .any(|child| pattern_binds(child, name, source))
}

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

fn parameter_type(function: Node<'_>, name: &str, source: &str) -> Option<Receiver> {
    let parameters = function.child_by_field_name("parameters")?;
    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        if parameter.kind() != "parameter" {
            continue;
        }
        let pattern = parameter.child_by_field_name("pattern")?;
        if binding_name(pattern, source).as_deref() == Some(name) {
            return type_name(parameter.child_by_field_name("type")?, source);
        }
    }
    None
}

/// The receiver type a type expression names: the named type behind references and the smart
/// pointers `Box`, `Rc` and `Arc`, or the trait of `dyn Trait` and `impl Trait`.
fn type_name(node: Node<'_>, source: &str) -> Option<Receiver> {
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
        "dynamic_type" | "abstract_type" => {
            trait_name(node.child_by_field_name("trait")?, source).map(Receiver::Trait)
        }
        // `dyn Trait + Send`: the first bound that names a type.
        "bounded_type" => first_type(node, source),
        _ => None,
    }
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

/// The trait a bound names: the first of `A + B`, without generic arguments.
fn trait_name(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() == "bounded_type" || node.kind() == "trait_bounds" {
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

fn constructor_type(value: Node<'_>, function_context: Node<'_>, source: &str) -> Option<Receiver> {
    match value.kind() {
        "call_expression" => {
            let mut function = value.child_by_field_name("function")?;
            while function.kind() == "generic_function" {
                function = function.child_by_field_name("function")?;
            }
            let name = match function.kind() {
                "identifier" | "type_identifier" => node_text(function, source).to_string(),
                "scoped_identifier" => {
                    let path = function.child_by_field_name("path")?;
                    path_segment(path, source)
                }
                _ => return None,
            };
            named_type(name, function_context, source)
        }
        "struct_expression" => {
            let name = value.child_by_field_name("name")?;
            let name = if name.kind() == "scoped_identifier" {
                path_segment(name, source)
            } else {
                node_text(name, source).to_string()
            };
            named_type(name, function_context, source)
        }
        "identifier" | "type_identifier" => named_type(
            node_text(value, source).to_string(),
            function_context,
            source,
        ),
        _ => None,
    }
}

fn named_type(name: String, function: Node<'_>, source: &str) -> Option<Receiver> {
    if name == "Self" {
        return self_type(function, source);
    }
    starts_type_name(&name).then_some(Receiver::Type(name))
}

fn resolve_self_type(ty: Receiver, function: Node<'_>, source: &str) -> Option<Receiver> {
    match ty {
        Receiver::Type(name) if name == "Self" => self_type(function, source),
        Receiver::Type(name) => {
            if let Some(bound) = generic_trait_bound(function, &name, source) {
                Some(Receiver::Trait(bound))
            } else {
                Some(Receiver::Type(name))
            }
        }
        other => Some(other),
    }
}

fn generic_trait_bound(function: Node<'_>, param_name: &str, source: &str) -> Option<String> {
    let mut current = Some(function);
    while let Some(node) = current {
        if matches!(node.kind(), "function_item" | "impl_item") {
            if let Some(bound) = trait_bound_on_item(node, param_name, source) {
                return Some(bound);
            }
        }
        current = node.parent();
    }
    None
}

fn trait_bound_on_item(item: Node<'_>, param_name: &str, source: &str) -> Option<String> {
    let mut cursor = item.walk();
    for child in item.named_children(&mut cursor) {
        if child.kind() == "type_parameters" {
            let mut p_cursor = child.walk();
            for param in child.named_children(&mut p_cursor) {
                if matches!(
                    param.kind(),
                    "type_parameter" | "constrained_type_parameter"
                ) {
                    let name_node = param
                        .child_by_field_name("name")
                        .or_else(|| param.child_by_field_name("left"))
                        .or_else(|| {
                            let mut c_cursor = param.walk();
                            param.named_children(&mut c_cursor).next()
                        });
                    if let Some(name_node) = name_node {
                        if node_text(name_node, source) == param_name {
                            if let Some(bounds) = param.child_by_field_name("bounds") {
                                if let Some(tr) = trait_name(bounds, source) {
                                    return Some(tr);
                                }
                            }
                            let mut c_cursor = param.walk();
                            for b in param.named_children(&mut c_cursor).skip(1) {
                                if let Some(tr) = trait_name(b, source) {
                                    return Some(tr);
                                }
                            }
                        }
                    }
                }
            }
        } else if child.kind() == "where_clause" {
            let mut w_cursor = child.walk();
            for pred in child.named_children(&mut w_cursor) {
                if pred.kind() == "where_predicate" {
                    let left_node = pred
                        .child_by_field_name("left")
                        .or_else(|| pred.child_by_field_name("type"))
                        .or_else(|| {
                            let mut c_cursor = pred.walk();
                            pred.named_children(&mut c_cursor).next()
                        });
                    if let Some(left_node) = left_node {
                        if node_text(left_node, source) == param_name {
                            if let Some(bounds) = pred.child_by_field_name("bounds") {
                                if let Some(tr) = trait_name(bounds, source) {
                                    return Some(tr);
                                }
                            }
                            let mut c_cursor = pred.walk();
                            for b in pred.named_children(&mut c_cursor).skip(1) {
                                if let Some(tr) = trait_name(b, source) {
                                    return Some(tr);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn self_type(function: Node<'_>, source: &str) -> Option<Receiver> {
    let mut current = function;
    while let Some(parent) = current.parent() {
        if parent.kind() == "impl_item" {
            return parent
                .child_by_field_name("type")
                .and_then(|ty| type_name(ty, source));
        }
        current = parent;
    }
    None
}

fn starts_type_name(name: &str) -> bool {
    name.starts_with(char::is_uppercase)
}

fn is_ancestor_block(ancestor: Node<'_>, descendant: Node<'_>) -> bool {
    let mut current = Some(descendant);
    while let Some(node) = current {
        if node.id() == ancestor.id() {
            return true;
        }
        current = node.parent();
    }
    false
}

fn block_depth(block: Node<'_>) -> usize {
    let mut depth = 0;
    let mut current = block.parent();
    while let Some(node) = current {
        if node.kind() == "block" {
            depth += 1;
        }
        current = node.parent();
    }
    depth
}
