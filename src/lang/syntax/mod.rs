//! Tree-sitter code intelligence shared by all languages.
//!
//! A language describes its grammar in a [`Syntax`] implementation of its own (one module per
//! language, for example `rust.rs`) and attaches it to its entry in [`super::LANGUAGES`]. The
//! tools built on top of this module (depth view, symbols, references, call graph) only use
//! the trait, so supporting another language needs no change to them.

pub mod python;
pub mod rust;

use tree_sitter::{Node, Parser, Tree};

/// A definition found in a syntax tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    /// Short kind shown in listings, such as `fn`, `struct` or `class`.
    pub kind: &'static str,
    pub name: String,
}

/// An import that gives a name another name in the importing file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    /// The name used in the file.
    pub alias: String,
    /// The last segment of what was imported: `b` for `use a::b as c`.
    pub original: String,
}

/// What the code intelligence tools need to know about one language.
pub trait Syntax: Sync {
    fn grammar(&self) -> tree_sitter::Language;

    /// Node kinds whose body adds one level of nesting in the depth view.
    fn block_kinds(&self) -> &'static [&'static str];

    /// Text shown in place of a block body that was left out; `first` and `last` are 1-based
    /// lines. It must not contain the lines themselves.
    fn elided(&self, first: usize, last: usize) -> String;

    /// The definition that `node` introduces, if it introduces one.
    fn definition(&self, node: Node<'_>, source: &str) -> Option<Definition>;

    /// Node kinds that hold a name: identifiers, type identifiers, field identifiers.
    fn identifier_kinds(&self) -> &'static [&'static str];

    /// Node kinds whose text is not code: comments and string literals.
    fn text_kinds(&self) -> &'static [&'static str];

    /// Whether a definition of this kind (see [`Definition::kind`]) has a body that can call
    /// other code.
    fn is_function(&self, kind: &str) -> bool {
        matches!(kind, "fn" | "def")
    }

    /// The alias that `node` declares, if it is an aliased import. Imports without `as` are not
    /// aliases. The default is for languages without renaming imports.
    fn alias(&self, _node: Node<'_>, _source: &str) -> Option<Alias> {
        None
    }

    /// The name a call expression calls (`foo` for `foo(1)`, `bar` for `x.bar()`), or `None`
    /// when `node` is not a call or its target is not a plain name.
    fn call_name(&self, node: Node<'_>, source: &str) -> Option<String>;
}

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
    let last = if end.column == 0 && end.row > start.row { end.row } else { end.row + 1 };
    (start.row + 1, last)
}

/// First line of a node's text without a trailing opening brace or colon: the signature of a
/// definition as written.
pub fn header(node: Node<'_>, source: &str) -> String {
    let first = node_text(node, source).lines().next().unwrap_or("").trim();
    first.trim_end_matches(['{', ':']).trim_end().to_string()
}
