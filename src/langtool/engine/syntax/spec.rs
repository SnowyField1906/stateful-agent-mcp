//! Tree-sitter code intelligence shared by all languages.
//!
//! A language describes its grammar in a [`Syntax`] implementation of its own (one module per
//! language, for example `rust.rs`) and is attached in [`super::registry`]. The tools built on
//! top of this module (folded views, symbols, references, call graph) only use the trait, so
//! supporting another language needs no change to them.

use tree_sitter::{Node, Parser, Tree};

use super::scope::Lineage;

/// A definition found in a syntax tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    /// Short kind shown in listings, such as `fn`, `struct` or `class`.
    pub kind: &'static str,
    pub name: String,
    /// For an `impl Trait for Type` block, the trait (`name` is then the type).
    pub trait_name: Option<String>,
}

/// A symbol written as a name with optional qualifiers: `put`, `Store::put`, `Store.put` or
/// `east/Token/parse`. The same text is accepted by every tool that takes a symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolSelector<'a> {
    /// Enclosing definitions, outermost first. They may skip levels (`east::parse` reaches
    /// `east::Token::parse`).
    pub qualifiers: Vec<&'a str>,
    pub name: &'a str,
}

impl<'a> SymbolSelector<'a> {
    pub fn parse(raw: &'a str) -> Self {
        let trimmed = raw.trim().trim_start_matches(['/', ':']);
        let parts: Vec<&str> = trimmed
            .split("::")
            .flat_map(|part| part.split('/'))
            .flat_map(|part| part.split('.'))
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        match parts.split_last() {
            Some((&name, qualifiers)) => Self {
                qualifiers: qualifiers.to_vec(),
                name,
            },
            None => Self {
                qualifiers: Vec::new(),
                name: trimmed,
            },
        }
    }

    /// Whether the qualifiers appear, in order and ignoring case, among the names of the
    /// enclosing definitions `scope`. Generic parameters are ignored.
    pub fn matches_scope(&self, scope: &Lineage) -> bool {
        let mut wanted = self.qualifiers.iter();
        let mut next = wanted.next();
        for frame in scope.frames() {
            let Some(target) = next else { break };
            let clean = frame.name.split('<').next().unwrap_or(&frame.name).trim();
            let matches_name = clean.eq_ignore_ascii_case(target);
            let matches_trait = frame.trait_name.as_deref().is_some_and(|t| {
                t.split('<')
                    .next()
                    .unwrap_or(t)
                    .trim()
                    .eq_ignore_ascii_case(target)
            });
            if matches_name || matches_trait {
                next = wanted.next();
            }
        }
        next.is_none()
    }
}

/// How a call was written, which decides the definitions it can reach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallKind {
    /// Nothing is known about the target beyond its name.
    Any,
    /// `foo()`: a function that is not a method.
    Free,
    /// A method of the enclosing type: `self.foo()`, `Self::foo()`.
    Own,
    /// A method on a receiver whose type the language adapter inferred.
    MethodOf(String),
    /// A method on a receiver typed as a trait object or `impl Trait`: the trait's declaration
    /// and every implementation of it.
    Dyn(String),
    /// `Type::foo()` or `module::foo()`: the last segment before the name.
    Path(String),
    /// `value.foo()` where the type of `value` is unknown: possible method targets.
    Method,
}

/// A call to a function by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallSite {
    pub name: String,
    pub kind: CallKind,
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

    /// The call that `node` makes with the way it was written. Calls that are not calls of
    /// functions, such as tuple-struct constructors, give `None`. The default knows only the name.
    fn call_site(&self, node: Node<'_>, source: &str) -> Option<CallSite> {
        self.call_name(node, source).map(|name| CallSite {
            name,
            kind: CallKind::Any,
        })
    }

    /// The kinds of definitions this language lists (see [`Definition::kind`]).
    fn kinds(&self) -> &'static [&'static str];

    /// Kinds of definitions that are listed only when a request names them (enum variants,
    /// struct fields).
    fn extra_kinds(&self) -> &'static [&'static str] {
        &[]
    }

    /// Node kinds of import statements.
    fn import_kinds(&self) -> &'static [&'static str] {
        &[]
    }
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
