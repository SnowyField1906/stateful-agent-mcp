//! Tree-sitter support for Python.

use tree_sitter::Node;

use super::{Alias, CallKind, CallSite, Definition, Syntax, node_text};

pub struct PythonSyntax;

impl Syntax for PythonSyntax {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }

    fn block_kinds(&self) -> &'static [&'static str] {
        &["block"]
    }

    fn elision(&self) -> &'static str {
        "..."
    }

    fn is_container(&self, node: Node<'_>) -> bool {
        node.kind() == "block"
            && node
                .parent()
                .is_some_and(|p| p.kind() == "class_definition")
    }

    fn is_doc(&self, node: Node<'_>, _source: &str) -> bool {
        node.kind() == "comment"
    }

    fn is_attribute(&self, node: Node<'_>) -> bool {
        node.kind() == "decorator"
    }

    fn unit_node<'a>(&self, node: Node<'a>) -> Node<'a> {
        match node.parent() {
            Some(parent) if parent.kind() == "decorated_definition" => parent,
            _ => node,
        }
    }

    fn doc_line(&self, node: Node<'_>, source: &str) -> Option<String> {
        first_docstring_line(node.child_by_field_name("body")?, source)
    }

    fn body_summary(&self, block: Node<'_>, source: &str) -> Option<String> {
        first_docstring_line(block, source).map(|line| format!("\"\"\"{line}\"\"\""))
    }

    fn definition(&self, node: Node<'_>, source: &str) -> Option<Definition> {
        let kind = match node.kind() {
            "function_definition" => "def",
            "class_definition" => "class",
            "assignment" if is_module_level(node) => {
                let left = node.child_by_field_name("left")?;
                if left.kind() != "identifier" {
                    return None;
                }
                return Some(Definition {
                    kind: "var",
                    name: node_text(left, source).to_string(),
                });
            }
            _ => return None,
        };
        let name = node.child_by_field_name("name")?;
        Some(Definition {
            kind,
            name: node_text(name, source).to_string(),
        })
    }

    fn identifier_kinds(&self) -> &'static [&'static str] {
        &["identifier"]
    }

    fn text_kinds(&self) -> &'static [&'static str] {
        &["comment", "string"]
    }

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
        match target.kind() {
            "identifier" => Some(CallSite {
                name: node_text(target, source).to_string(),
                kind: CallKind::Free,
            }),
            "attribute" => {
                let name = node_text(target.child_by_field_name("attribute")?, source).to_string();
                let own = target
                    .child_by_field_name("object")
                    .is_some_and(|o| matches!(node_text(o, source), "self" | "cls"));
                Some(CallSite {
                    name,
                    kind: if own { CallKind::Own } else { CallKind::Any },
                })
            }
            _ => None,
        }
    }

    fn kinds(&self) -> &'static [&'static str] {
        &["def", "class", "var"]
    }

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
}

/// The first line of the docstring that starts `block`, if its first statement is a string.
fn first_docstring_line(block: Node<'_>, source: &str) -> Option<String> {
    let mut cursor = block.walk();
    let stmt = block
        .named_children(&mut cursor)
        .find(|n| n.kind() != "comment")?;
    if stmt.kind() != "expression_statement" {
        return None;
    }
    let literal = stmt.named_child(0).filter(|n| n.kind() == "string")?;
    let text = node_text(literal, source).trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let body = ["\"\"\"", "'''", "\"", "'"]
        .iter()
        .find_map(|q| text.strip_prefix(*q))?;
    let line = body.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_end_matches(['"', '\'']).trim();
    (!line.is_empty()).then(|| line.to_string())
}

/// Whether an assignment is a statement directly in the module body.
fn is_module_level(node: Node<'_>) -> bool {
    node.parent()
        .filter(|p| p.kind() == "expression_statement")
        .and_then(|p| p.parent())
        .is_some_and(|g| g.kind() == "module")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langtool::engine::syntax::parse;

    const SOURCE: &str = r#"# helper() in a comment
LIMIT = 3

class Point:
    def __init__(self):
        s = "helper()"
        helper(s)

@decorator
def helper(s):
    p = Point()
    p.draw()
    util.run()
"#;

    fn all_nodes(tree: &tree_sitter::Tree) -> Vec<Node<'_>> {
        fn walk<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
            out.push(node);
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk(child, out);
            }
        }
        let mut out = Vec::new();
        walk(tree.root_node(), &mut out);
        out
    }

    #[test]
    fn lists_definitions() {
        let tree = parse(&PythonSyntax, SOURCE).expect("parse");
        let found: Vec<(&str, String)> = all_nodes(&tree)
            .into_iter()
            .filter_map(|n| PythonSyntax.definition(n, SOURCE))
            .map(|d| (d.kind, d.name))
            .collect();
        assert_eq!(
            found,
            [
                ("var", "LIMIT".to_string()),
                ("class", "Point".to_string()),
                ("def", "__init__".to_string()),
                ("def", "helper".to_string()),
            ]
        );
    }

    #[test]
    fn resolves_call_names() {
        let tree = parse(&PythonSyntax, SOURCE).expect("parse");
        let mut calls: Vec<String> = all_nodes(&tree)
            .into_iter()
            .filter_map(|n| PythonSyntax.call_name(n, SOURCE))
            .collect();
        calls.sort();
        assert_eq!(calls, ["Point", "draw", "helper", "run"]);
    }

    #[test]
    fn marks_comments_and_strings_as_text() {
        let tree = parse(&PythonSyntax, SOURCE).expect("parse");
        let kinds = PythonSyntax.text_kinds();
        let in_text = all_nodes(&tree)
            .into_iter()
            .filter(|n| kinds.contains(&n.kind()))
            .filter(|n| node_text(*n, SOURCE).contains("helper()"))
            .count();
        assert_eq!(in_text, 2);
    }
}
