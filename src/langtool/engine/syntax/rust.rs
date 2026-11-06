//! Tree-sitter support for Rust.

use tree_sitter::Node;

use super::{Alias, CallKind, CallSite, Definition, Syntax, line_span, node_text};

mod receiver;

pub struct RustSyntax;

impl Syntax for RustSyntax {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_rust::LANGUAGE.into()
    }

    fn block_kinds(&self) -> &'static [&'static str] {
        &[
            "declaration_list",
            "field_declaration_list",
            "enum_variant_list",
            "block",
            "match_block",
        ]
    }

    fn elision(&self) -> &'static str {
        "{ ... }"
    }

    fn is_container(&self, node: Node<'_>) -> bool {
        node.kind() == "declaration_list"
    }

    fn is_doc(&self, node: Node<'_>, source: &str) -> bool {
        let text = node_text(node, source);
        match node.kind() {
            "line_comment" => text.starts_with("///") && !text.starts_with("////"),
            "block_comment" => {
                text.starts_with("/**") && !text.starts_with("/***") && text != "/**/"
            }
            _ => false,
        }
    }

    fn is_attribute(&self, node: Node<'_>) -> bool {
        node.kind() == "attribute_item"
    }

    fn doc_line(&self, node: Node<'_>, source: &str) -> Option<String> {
        let mut first = None;
        let mut cur = node;
        while let Some(prev) = cur.prev_sibling() {
            if line_span(prev).1 + 1 != line_span(cur).0 {
                break;
            }
            match prev.kind() {
                "attribute_item" => {}
                "line_comment" if self.is_doc(prev, source) => {
                    let text = node_text(prev, source).trim_start_matches('/').trim();
                    if !text.is_empty() {
                        first = Some(text.to_string());
                    }
                }
                _ => break,
            }
            cur = prev;
        }
        first
    }

    fn definition(&self, node: Node<'_>, source: &str) -> Option<Definition> {
        let kind = match node.kind() {
            "function_item" | "function_signature_item" => "fn",
            "struct_item" => "struct",
            "enum_item" => "enum",
            "enum_variant" => "variant",
            "field_declaration" => "field",
            "union_item" => "union",
            "trait_item" => "trait",
            "type_item" => "type",
            "const_item" => "const",
            "static_item" => "static",
            "mod_item" => "mod",
            "macro_definition" => "macro",
            "impl_item" => {
                let target = node.child_by_field_name("type")?;
                return Some(Definition {
                    kind: "impl",
                    name: node_text(target, source).to_string(),
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
        &["identifier", "type_identifier", "field_identifier"]
    }

    fn text_kinds(&self) -> &'static [&'static str] {
        &[
            "line_comment",
            "block_comment",
            "string_literal",
            "raw_string_literal",
            "char_literal",
        ]
    }

    fn call_name(&self, node: Node<'_>, source: &str) -> Option<String> {
        if node.kind() != "call_expression" {
            return None;
        }
        target_name(node.child_by_field_name("function")?, source)
    }

    fn call_site(&self, node: Node<'_>, source: &str) -> Option<CallSite> {
        if node.kind() != "call_expression" {
            return None;
        }
        let site = target_site(node.child_by_field_name("function")?, source)?;
        // `Some(x)`, `Err(e)` and `Enum::Variant(x)` construct values; functions are snake_case.
        if site.name.starts_with(char::is_uppercase) {
            return None;
        }
        Some(site)
    }

    fn kinds(&self) -> &'static [&'static str] {
        &[
            "fn", "struct", "enum", "union", "trait", "type", "const", "static", "mod", "macro",
            "impl",
        ]
    }

    fn extra_kinds(&self) -> &'static [&'static str] {
        &["variant", "field"]
    }

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
}

/// The last segment of a path: `b` for `a::b` and for `b`.
fn last_segment(node: Node<'_>, source: &str) -> String {
    let name = match node.kind() {
        "scoped_identifier" => node.child_by_field_name("name").unwrap_or(node),
        _ => node,
    };
    node_text(name, source).to_string()
}

/// The name a call target ends in and how the call was written.
fn target_site(node: Node<'_>, source: &str) -> Option<CallSite> {
    let site = |name: Node<'_>, kind: CallKind| CallSite {
        name: node_text(name, source).to_string(),
        kind,
    };
    match node.kind() {
        "identifier" => Some(site(node, CallKind::Free)),
        "scoped_identifier" => {
            let name = node.child_by_field_name("name")?;
            let kind = match node.child_by_field_name("path") {
                Some(path) => match path_segment(path, source).as_str() {
                    "Self" => CallKind::Own,
                    "self" | "crate" | "super" => CallKind::Free,
                    other => CallKind::Path(other.to_string()),
                },
                None => CallKind::Free,
            };
            Some(site(name, kind))
        }
        "field_expression" => {
            let name = node.child_by_field_name("field")?;
            let receiver = node.child_by_field_name("value")?;
            let kind = if node_text(receiver, source) == "self" {
                CallKind::Own
            } else if let Some(ty) = receiver::infer_type(node, source) {
                CallKind::MethodOf(ty)
            } else {
                CallKind::Method
            };
            Some(site(name, kind))
        }
        "generic_function" => target_site(node.child_by_field_name("function")?, source),
        _ => None,
    }
}

/// The last segment of the path in front of a call's name; the type of a generic type.
fn path_segment(node: Node<'_>, source: &str) -> String {
    let node = match node.kind() {
        "generic_type" => node.child_by_field_name("type").unwrap_or(node),
        _ => node,
    };
    last_segment(node, source)
}

/// The plain name a call target ends in: `foo` for `foo`, `a::foo`, `foo::<T>` and `x.foo`.
fn target_name(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" => Some(node_text(node, source).to_string()),
        "scoped_identifier" => target_name(node.child_by_field_name("name")?, source),
        "field_expression" => target_name(node.child_by_field_name("field")?, source),
        "generic_function" => target_name(node.child_by_field_name("function")?, source),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langtool::engine::syntax::parse;

    const SOURCE: &str = r#"
// helper() in a comment
struct Point { x: i32 }

enum Shape { Dot, Line }

trait Draw { fn draw(&self); }

impl Point {
    fn new() -> Self {
        let s = "helper()";
        helper(s);
        Point { x: 0 }
    }
}

fn helper(_s: &str) {
    let p = Point::new();
    p.draw::<u8>();
    util::run();
}

const LIMIT: usize = 3;
static NAME: &str = "n";
type Alias = Point;
mod inner {}
macro_rules! mk { () => {}; }
"#;

    fn walk<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
        out.push(node);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, out);
        }
    }

    fn nodes() -> (tree_sitter::Tree, Vec<usize>) {
        let tree = parse(&RustSyntax, SOURCE).expect("parse");
        (tree, Vec::new())
    }

    #[test]
    fn lists_definitions() {
        let (tree, _) = nodes();
        let mut all = Vec::new();
        walk(tree.root_node(), &mut all);
        let found: Vec<(&str, String)> = all
            .iter()
            .filter_map(|n| RustSyntax.definition(*n, SOURCE))
            .map(|d| (d.kind, d.name))
            .collect();
        for expected in [
            ("struct", "Point"),
            ("enum", "Shape"),
            ("trait", "Draw"),
            ("impl", "Point"),
            ("fn", "new"),
            ("fn", "helper"),
            ("fn", "draw"),
            ("const", "LIMIT"),
            ("static", "NAME"),
            ("type", "Alias"),
            ("mod", "inner"),
            ("macro", "mk"),
        ] {
            assert!(
                found
                    .iter()
                    .any(|(k, n)| *k == expected.0 && n == expected.1),
                "missing {expected:?} in {found:?}"
            );
        }
    }

    #[test]
    fn resolves_call_names() {
        let (tree, _) = nodes();
        let mut all = Vec::new();
        walk(tree.root_node(), &mut all);
        let mut calls: Vec<String> = all
            .iter()
            .filter_map(|n| RustSyntax.call_name(*n, SOURCE))
            .collect();
        calls.sort();
        assert_eq!(calls, ["draw", "helper", "new", "run"]);
    }

    #[test]
    fn marks_comments_and_strings_as_text() {
        let (tree, _) = nodes();
        let mut all = Vec::new();
        walk(tree.root_node(), &mut all);
        let text_kinds = RustSyntax.text_kinds();
        let in_text = all
            .iter()
            .filter(|n| text_kinds.contains(&n.kind()))
            .filter(|n| node_text(**n, SOURCE).contains("helper()"))
            .count();
        assert_eq!(in_text, 2);
    }
}
