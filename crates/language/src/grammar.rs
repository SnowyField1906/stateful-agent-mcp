//! Grammar: how the source of a language is parsed and shown.

use tree_sitter::Node;

/// What an identifier names. The tools report it as the role of a reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ident {
    /// A variable, function, module or any other name of a value.
    Value,
    /// A type or a trait.
    Type,
    /// A field of a value.
    Field,
}

/// The Tree-sitter grammar of a language and what the folded views and the name searches need to
/// tell apart in its trees.
pub trait Grammar {
    /// The Tree-sitter grammar.
    fn grammar(&self) -> tree_sitter::Language;

    /// Node kinds whose body adds one level of nesting in the folded view.
    fn block_kinds(&self) -> &'static [&'static str];

    /// Text shown in place of a block body that was left out. The folded row already carries
    /// its line range, so the text holds no line numbers.
    fn elision(&self) -> &'static str;

    /// Whether the block `node` only holds declarations (an `impl`, `trait` or `mod` body, a
    /// class body). Containers add no nesting in the folded view, so level 0 lists methods.
    fn is_container(&self, _node: Node<'_>) -> bool {
        false
    }

    /// Whether `node` is a documentation comment.
    fn is_doc(&self, _node: Node<'_>, _source: &str) -> bool {
        false
    }

    /// Whether `node` is an attribute or decorator written above a definition.
    fn is_attribute(&self, _node: Node<'_>) -> bool {
        false
    }

    /// The node that spans a definition together with the syntax wrapped around it (Python's
    /// `decorated_definition`).
    fn unit_node<'a>(&self, node: Node<'a>) -> Node<'a> {
        node
    }

    /// First line of the documentation of the definition `node`, without comment markers.
    fn doc_line(&self, _node: Node<'_>, _source: &str) -> Option<String> {
        None
    }

    /// For languages that document inside the body (Python docstrings): the first documentation
    /// line of the block `block`, as it is shown in place of the folded body.
    fn body_summary(&self, _block: Node<'_>, _source: &str) -> Option<String> {
        None
    }

    /// What `node` names when it is an identifier, `None` for any other node.
    fn identifier(&self, node: Node<'_>) -> Option<Ident>;

    /// Node kinds whose text is not code: comments and string literals.
    fn text_kinds(&self) -> &'static [&'static str];
}
