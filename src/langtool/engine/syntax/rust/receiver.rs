//! Conservative local receiver-type inference for Rust method calls.
//!
//! This is syntax-based, not compiler type checking. It resolves named types from annotations
//! and local constructors; bindings introduced by patterns without a known type stay unknown.

use tree_sitter::Node;

use super::{node_text, path_segment};

/// Infer the type of the receiver in a `field_expression` such as `value.method`.
pub(super) fn infer_type(field: Node<'_>, source: &str) -> Option<String> {
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
) -> Option<Option<String>> {
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
    bindings: &mut Vec<(usize, usize, Option<String>)>,
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

fn parameter_type(function: Node<'_>, name: &str, source: &str) -> Option<String> {
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

fn type_name(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(node_text(node, source).to_string()),
        "scoped_type_identifier" => {
            let name = node.child_by_field_name("name")?;
            Some(node_text(name, source).to_string())
        }
        "generic_type" => type_name(node.child_by_field_name("type")?, source),
        "reference_type" | "pointer_type" => type_name(node.child_by_field_name("type")?, source),
        _ => None,
    }
}

fn constructor_type(value: Node<'_>, function_context: Node<'_>, source: &str) -> Option<String> {
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

fn named_type(name: String, function: Node<'_>, source: &str) -> Option<String> {
    let name = if name == "Self" {
        self_type(function, source)?
    } else {
        name
    };
    starts_type_name(&name).then_some(name)
}

fn resolve_self_type(ty: String, function: Node<'_>, source: &str) -> Option<String> {
    if ty == "Self" {
        self_type(function, source)
    } else {
        Some(ty)
    }
}

fn self_type(function: Node<'_>, source: &str) -> Option<String> {
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
