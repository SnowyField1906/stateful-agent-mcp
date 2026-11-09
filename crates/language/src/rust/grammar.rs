//! Rust grammar: parsing, doc comments and the folded view.

use tree_sitter::Node;

use crate::{Grammar, Ident, line_span, node_text};

use super::RustSyntax;

impl Grammar for RustSyntax {
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

    /// The first line of the leading `//!` comment.
    fn file_doc(&self, _root: Node<'_>, source: &str) -> Option<String> {
        for line in source
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let text = line.strip_prefix("//!")?.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        None
    }

    fn identifier(&self, node: Node<'_>) -> Option<Ident> {
        match node.kind() {
            "identifier" => Some(Ident::Value),
            "type_identifier" => Some(Ident::Type),
            "field_identifier" => Some(Ident::Field),
            _ => None,
        }
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
}
