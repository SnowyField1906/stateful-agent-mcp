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

    /// First line of the documentation of the whole file (`root` is its syntax tree), without
    /// comment markers.
    fn file_doc(&self, _root: Node<'_>, _source: &str) -> Option<String> {
        None
    }

    /// What `node` names when it is an identifier, `None` for any other node.
    fn identifier(&self, node: Node<'_>) -> Option<Ident>;

    /// Whether the identifier `node` introduces a local name: a variable of a `let` or a loop, a
    /// parameter, a name bound by a pattern. Such a name says nothing about the definitions of
    /// the same name elsewhere. The default is that none does.
    fn is_binding(&self, _node: Node<'_>) -> bool {
        false
    }

    /// Whether the identifier `node` is written as a segment of a path (`a::b`), which names a
    /// definition, never a local variable. The default is that none is.
    fn is_path(&self, _node: Node<'_>) -> bool {
        false
    }

    /// Node kinds whose text is not code: comments and string literals.
    fn text_kinds(&self) -> &'static [&'static str];
}

/// Whether `child` is the node that the field `name` of `parent` holds.
pub(crate) fn is_field(parent: Node<'_>, name: &str, child: Node<'_>) -> bool {
    parent
        .child_by_field_name(name)
        .is_some_and(|node| node.id() == child.id())
}
