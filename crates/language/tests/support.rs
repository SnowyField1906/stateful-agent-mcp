//! What the contract tests share: the registered syntaxes, the sample source file of each language
//! and a walk over its syntax tree.

use std::fs;
use std::path::PathBuf;

use stateful_agent_language::{Language, Syntax, node_text, parse, registry};
use tree_sitter::{Node, Tree};

/// The syntax of every registered language.
pub fn syntaxes() -> Vec<&'static dyn Syntax> {
    let syntaxes: Vec<_> = registry::supported_languages()
        .filter_map(registry::of)
        .collect();
    assert!(!syntaxes.is_empty(), "no language is registered");
    syntaxes
}

/// The name of the language, for the messages of a failed check.
pub fn name(syntax: &dyn Syntax) -> &'static str {
    syntax.language().name
}

/// Where the sample of `language` lives: `tests/fixtures/<name>/sample.<extension>`.
fn sample_path(language: &Language) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(language.name)
        .join(format!("sample.{}", language.extensions[0]))
}

/// A source file written for the contract tests, and its syntax tree.
pub struct Sample {
    pub source: String,
    pub tree: Tree,
}

impl Sample {
    /// Read and parse the sample of the language. A language without one fails here, and the
    /// message says what the file has to show.
    pub fn of(syntax: &dyn Syntax) -> Self {
        let path = sample_path(syntax.language());
        let source = fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "{}: write {}, valid code of the language with a documented type that holds a \
                 method, a function, a comment, a call of a function, a call on `self`, a call \
                 through a path or on a value, and an import with an alias",
                name(syntax),
                path.display()
            )
        });
        let tree = parse(syntax, &source)
            .unwrap_or_else(|| panic!("{}: the grammar does not load", name(syntax)));
        Self { source, tree }
    }

    /// Every node of the tree, parents before their children.
    pub fn nodes(&self) -> Vec<Node<'_>> {
        let mut nodes = Vec::new();
        let mut cursor = self.tree.walk();
        loop {
            nodes.push(cursor.node());
            if cursor.goto_first_child() {
                continue;
            }
            while !cursor.goto_next_sibling() {
                if !cursor.goto_parent() {
                    return nodes;
                }
            }
        }
    }

    pub fn text(&self, node: Node<'_>) -> &str {
        node_text(node, &self.source)
    }
}

/// Whether `node` is `ancestor` or lies below it.
pub fn is_inside(node: Node<'_>, ancestor: Node<'_>) -> bool {
    std::iter::successors(Some(node), |node| node.parent()).any(|node| node.id() == ancestor.id())
}
