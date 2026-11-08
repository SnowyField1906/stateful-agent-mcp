//! Python imports: the names `import` and `from .. import` bind.

use tree_sitter::Node;

use crate::{Alias, Import, Imports, node_text};

use super::PythonSyntax;

impl Imports for PythonSyntax {
    fn import_kinds(&self) -> &'static [&'static str] {
        &["import_statement", "import_from_statement"]
    }

    fn alias(&self, node: Node<'_>, source: &str) -> Option<Alias> {
        if node.kind() != "aliased_import" {
            return None;
        }
        let alias = node_text(node.child_by_field_name("alias")?, source);
        let name = node_text(node.child_by_field_name("name")?, source);
        let original = name.rsplit('.').next().unwrap_or(name);
        Some(Alias {
            alias: alias.to_string(),
            original: original.to_string(),
        })
    }

    fn imports(&self, node: Node<'_>, source: &str) -> Vec<Import> {
        match node.kind() {
            "import_statement" => import_names(node, source),
            "import_from_statement" => from_names(node, source),
            _ => Vec::new(),
        }
    }

    fn module_entry_stems(&self) -> &'static [&'static str] {
        &["__init__"]
    }
}

/// `["a", "b"]` for `a.b`.
fn dotted_path(node: Node<'_>, source: &str) -> Vec<String> {
    node_text(node, source)
        .split('.')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

/// The names that `import a.b, c as d` binds: `a` for `a.b`, `d` for `c as d`.
fn import_names(node: Node<'_>, source: &str) -> Vec<Import> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for name in node.children_by_field_name("name", &mut cursor) {
        match name.kind() {
            "dotted_name" => {
                let path = dotted_path(name, source);
                if let Some(local) = path.first().cloned() {
                    out.push(Import {
                        local,
                        path,
                        relative: false,
                    });
                }
            }
            "aliased_import" => {
                let (Some(target), Some(alias)) = (
                    name.child_by_field_name("name"),
                    name.child_by_field_name("alias"),
                ) else {
                    continue;
                };
                out.push(Import {
                    local: node_text(alias, source).to_string(),
                    path: dotted_path(target, source),
                    relative: false,
                });
            }
            _ => {}
        }
    }
    out
}

/// The names that `from a.b import c as d, e` binds, and the wildcard of `from a import *`.
fn from_names(node: Node<'_>, source: &str) -> Vec<Import> {
    let (relative, base) = match node.child_by_field_name("module_name") {
        Some(module) if module.kind() == "relative_import" => {
            let mut cursor = module.walk();
            let path = module
                .named_children(&mut cursor)
                .find(|child| child.kind() == "dotted_name")
                .map(|name| dotted_path(name, source))
                .unwrap_or_default();
            (true, path)
        }
        Some(module) => (false, dotted_path(module, source)),
        None => (false, Vec::new()),
    };
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "wildcard_import" {
            out.push(Import {
                local: "*".to_string(),
                path: base.clone(),
                relative,
            });
        }
    }
    let mut cursor = node.walk();
    for name in node.children_by_field_name("name", &mut cursor) {
        let (local, tail) = match name.kind() {
            "dotted_name" => {
                let tail = dotted_path(name, source);
                let Some(local) = tail.last().cloned() else {
                    continue;
                };
                (local, tail)
            }
            "aliased_import" => {
                let (Some(target), Some(alias)) = (
                    name.child_by_field_name("name"),
                    name.child_by_field_name("alias"),
                ) else {
                    continue;
                };
                (
                    node_text(alias, source).to_string(),
                    dotted_path(target, source),
                )
            }
            _ => continue,
        };
        let mut path = base.clone();
        path.extend(tail);
        out.push(Import {
            local,
            path,
            relative,
        });
    }
    out
}
