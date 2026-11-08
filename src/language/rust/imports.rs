//! Rust imports: the names a `use` binds and the module tree they live in.

use tree_sitter::Node;

use crate::language::{Alias, Import, Imports, node_text};

use super::RustSyntax;
use super::nodes::last_segment;

impl Imports for RustSyntax {
    fn import_kinds(&self) -> &'static [&'static str] {
        &["use_declaration"]
    }

    fn alias(&self, node: Node<'_>, source: &str) -> Option<Alias> {
        if node.kind() != "use_as_clause" {
            return None;
        }
        let alias = node_text(node.child_by_field_name("alias")?, source);
        if alias == "_" {
            return None;
        }
        let original = last_segment(node.child_by_field_name("path")?, source);
        Some(Alias {
            alias: alias.to_string(),
            original,
        })
    }

    fn imports(&self, node: Node<'_>, source: &str) -> Vec<Import> {
        let mut out = Vec::new();
        if node.kind() == "use_declaration"
            && let Some(argument) = node.child_by_field_name("argument")
        {
            collect_use(argument, &[], source, &mut out);
        }
        out
    }

    fn is_path_keyword(&self, segment: &str) -> bool {
        matches!(segment, "crate" | "self" | "super")
    }

    fn module_entry_stems(&self) -> &'static [&'static str] {
        &["mod", "lib", "main"]
    }
}

/// The imports that the use tree `node` binds below the path `prefix`.
fn collect_use(node: Node<'_>, prefix: &[String], source: &str, out: &mut Vec<Import>) {
    match node.kind() {
        "identifier" | "scoped_identifier" | "crate" | "self" | "super" => {
            let path = joined(prefix, node, source);
            if let Some(local) = path.last().cloned() {
                out.push(Import {
                    local,
                    path,
                    relative: false,
                });
            }
        }
        "use_as_clause" => {
            let (Some(path), Some(alias)) = (
                node.child_by_field_name("path"),
                node.child_by_field_name("alias"),
            ) else {
                return;
            };
            let local = node_text(alias, source).to_string();
            if local != "_" {
                out.push(Import {
                    local,
                    path: joined(prefix, path, source),
                    relative: false,
                });
            }
        }
        "use_wildcard" => {
            let mut path = prefix.to_vec();
            if let Some(inner) = node.named_child(0) {
                path.extend(segments(inner, source));
            }
            out.push(Import {
                local: "*".to_string(),
                path,
                relative: false,
            });
        }
        "scoped_use_list" => {
            let mut path = prefix.to_vec();
            if let Some(inner) = node.child_by_field_name("path") {
                path.extend(segments(inner, source));
            }
            if let Some(list) = node.child_by_field_name("list") {
                collect_use(list, &path, source, out);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_use(child, prefix, source, out);
            }
        }
        _ => {}
    }
}

/// `prefix` followed by the segments of `node`; `use a::{self}` imports `a`.
fn joined(prefix: &[String], node: Node<'_>, source: &str) -> Vec<String> {
    let mut path = prefix.to_vec();
    path.extend(segments(node, source));
    if path.len() > 1 && path.last().is_some_and(|last| last == "self") {
        path.pop();
    }
    path
}

fn segments(node: Node<'_>, source: &str) -> Vec<String> {
    node_text(node, source)
        .split("::")
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}
