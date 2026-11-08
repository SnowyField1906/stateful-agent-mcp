//! Reading syntax trees: parsing, node text and line spans, shared by every language.

use tree_sitter::{Node, Parser, Tree};

use super::Syntax;

/// Parse `source`. `None` only when the grammar cannot be loaded or parsing is cancelled.
pub fn parse(syntax: &dyn Syntax, source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&syntax.grammar()).ok()?;
    parser.parse(source, None)
}

pub fn node_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

/// 1-based first and last line of a node. A node that ends at the start of a line does not
/// count that line.
pub fn line_span(node: Node<'_>) -> (usize, usize) {
    let (start, end) = (node.start_position(), node.end_position());
    let last = if end.column == 0 && end.row > start.row {
        end.row
    } else {
        end.row + 1
    };
    (start.row + 1, last)
}

/// 1-based first and last line of the unit a definition forms: the definition (with any syntax
/// wrapped around it) and the doc comments and attributes written directly above it, without a
/// blank line in between.
pub fn unit_span(syntax: &dyn Syntax, node: Node<'_>, source: &str) -> (usize, usize) {
    let unit = syntax.unit_node(node);
    let (mut first, last) = line_span(unit);
    let mut cur = unit;
    while let Some(prev) = cur.prev_sibling() {
        let (prev_first, prev_last) = line_span(prev);
        let attached = prev_last + 1 == first
            && (syntax.is_doc(prev, source) || syntax.is_attribute(prev))
            && starts_line(source, prev.start_byte());
        if !attached {
            break;
        }
        first = prev_first;
        cur = prev;
    }
    (first, last)
}

/// Whether only blanks come before `byte` on its line.
fn starts_line(source: &str, byte: usize) -> bool {
    source.as_bytes()[..byte]
        .iter()
        .rev()
        .take_while(|&&b| b != b'\n')
        .all(|&b| b == b' ' || b == b'\t')
}

/// Longest signature shown, in characters.
const SIGNATURE_CHARS: usize = 240;

/// The signature of a definition as written, on one line: the text before its body (so a
/// parameter list over several lines is kept), or the first line when it has no body.
pub fn header(node: Node<'_>, source: &str) -> String {
    let text = match node.child_by_field_name("body") {
        Some(body) => source
            .get(node.start_byte()..body.start_byte())
            .unwrap_or(""),
        None => node_text(node, source).lines().next().unwrap_or(""),
    };
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let tidy = joined
        .replace("( ", "(")
        .replace(", )", ")")
        .replace(", ]", "]");
    let tidy = tidy.trim_end_matches(['{', ':', ',']).trim_end();
    if tidy.chars().count() <= SIGNATURE_CHARS {
        return tidy.to_string();
    }
    let cut: String = tidy.chars().take(SIGNATURE_CHARS).collect();
    format!("{cut}…")
}
