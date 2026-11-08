//! Python calls: the name a call ends in and how it was written.

use tree_sitter::Node;

use crate::{CallKind, CallSite, Calls, Declarations, node_text};

use super::PythonSyntax;

impl Calls for PythonSyntax {
    fn call_name(&self, node: Node<'_>, source: &str) -> Option<String> {
        if node.kind() != "call" {
            return None;
        }
        let target = node.child_by_field_name("function")?;
        match target.kind() {
            "identifier" => Some(node_text(target, source).to_string()),
            "attribute" => {
                let name = target.child_by_field_name("attribute")?;
                Some(node_text(name, source).to_string())
            }
            _ => None,
        }
    }

    fn call_site(&self, node: Node<'_>, source: &str) -> Option<CallSite> {
        if node.kind() != "call" {
            return None;
        }
        let target = node.child_by_field_name("function")?;
        let site =
            |name: String, kind: CallKind, root: Option<String>| CallSite { name, kind, root };
        match target.kind() {
            "identifier" => Some(site(
                node_text(target, source).to_string(),
                CallKind::Free,
                None,
            )),
            "attribute" => {
                let name = node_text(target.child_by_field_name("attribute")?, source).to_string();
                let object = target.child_by_field_name("object")?;
                let obj_text = node_text(object, source);
                if matches!(obj_text, "self" | "cls") {
                    return Some(site(name, CallKind::Own, None));
                }
                if object.kind() == "identifier" {
                    if self.is_type_name(obj_text) || is_imported_module(object, obj_text, source) {
                        return Some(site(
                            name,
                            CallKind::Path(obj_text.to_string()),
                            Some(obj_text.to_string()),
                        ));
                    }
                    if let Some(type_name) = parameter_annotation(object, obj_text, source) {
                        return Some(site(name, CallKind::MethodOf(type_name), None));
                    }
                }
                Some(site(name, CallKind::Method, chain_root(object, source)))
            }
            _ => None,
        }
    }
}

fn is_imported_module(node: Node<'_>, name: &str, source: &str) -> bool {
    let mut current = node;
    while let Some(parent) = current.parent() {
        current = parent;
    }
    let mut cursor = current.walk();
    for stmt in current.named_children(&mut cursor) {
        if stmt.kind() == "import_statement" {
            let mut c = stmt.walk();
            for dotted in stmt.named_children(&mut c) {
                if dotted.kind() == "dotted_name" && node_text(dotted, source) == name {
                    return true;
                }
                if dotted.kind() == "aliased_import"
                    && dotted
                        .child_by_field_name("alias")
                        .is_some_and(|a| node_text(a, source) == name)
                {
                    return true;
                }
            }
        }
    }
    false
}

/// The name an attribute chain starts with: `os` for `os.path`.
fn chain_root(node: Node<'_>, source: &str) -> Option<String> {
    let mut current = node;
    while current.kind() == "attribute" {
        current = current.child_by_field_name("object")?;
    }
    (current.kind() == "identifier").then(|| node_text(current, source).to_string())
}

fn parameter_annotation(node: Node<'_>, param_name: &str, source: &str) -> Option<String> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.kind() == "function_definition" {
            let params = parent.child_by_field_name("parameters")?;
            let mut cursor = params.walk();
            for param in params.named_children(&mut cursor) {
                if matches!(param.kind(), "typed_parameter" | "typed_default_parameter") {
                    let name_node = param.child_by_field_name("name")?;
                    if node_text(name_node, source) == param_name {
                        let ty_node = param.child_by_field_name("type")?;
                        return Some(node_text(ty_node, source).to_string());
                    }
                }
            }
            return None;
        }
        current = parent;
    }
    None
}
