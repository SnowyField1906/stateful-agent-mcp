//! Symbol listing: the definitions in a file with their line span, parent scope and signature.
//!
//! Only the [`Syntax`] trait is used. Reading files and paginating results belong to the tool
//! handler.

use tree_sitter::Node;

use super::name_match::ranked_names;
use super::scope::{Frame, Lineage};
use super::selector::SymbolSelector;
use crate::language::{Kind, Syntax, header, parse, unit_span};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub kind: &'static Kind,
    pub name: String,
    /// For an `impl Trait for Type` block, the trait (`name` is then the type).
    pub trait_name: Option<String>,
    /// Whether the body of the definition is test code.
    pub test: bool,
    /// The enclosing definitions, outermost first; shown as a qualified path (`east::Token`).
    /// Empty at the top level.
    pub scope: Lineage,
    /// 1-based first and last line, including the doc comments, attributes and decorators
    /// written directly above the definition.
    pub first: usize,
    pub last: usize,
    /// 1-based line the definition itself starts on, below its doc comments and attributes. For a
    /// decorated Python function this is the `def` line.
    pub head: usize,
    /// First line of the definition as written, without a trailing `{` or `:`.
    pub signature: String,
    /// First line of the documentation, empty when there is none.
    pub doc: String,
    /// Source lines below the signature row (`  N<tab>text`), set by [`attach_bodies`].
    pub body: Option<String>,
}

impl Symbol {
    /// The frame this definition opens for the symbols written inside it.
    fn frame(&self) -> Frame {
        Frame {
            kind: self.kind,
            name: self.name.clone(),
            trait_name: self.trait_name.clone(),
            test: self.test,
        }
    }
}

/// All definitions in `source`, in source order. `None` when it cannot be parsed.
pub fn symbols(syntax: &dyn Syntax, source: &str) -> Option<Vec<Symbol>> {
    let tree = parse(syntax, source)?;
    let mut out = Vec::new();
    walk(
        tree.root_node(),
        &mut Lineage::default(),
        syntax,
        source,
        &mut out,
    );
    Some(out)
}

/// Keep symbols whose name contains the name of the `query` selector (case-insensitive), whose
/// enclosing definitions match its qualifiers (`Store::put`), and whose kind equals `kind`.
pub fn matches(symbol: &Symbol, query: Option<&str>, kind: Option<&str>) -> bool {
    query.is_none_or(|q| {
        let selector = SymbolSelector::parse(q);
        let name_matches = symbol
            .name
            .to_lowercase()
            .contains(&selector.name.to_lowercase())
            && selector.matches_scope(&symbol.scope);
        if name_matches {
            return true;
        }
        if let Some(tr) = &symbol.trait_name
            && tr.to_lowercase().contains(&selector.name.to_lowercase())
            && selector.matches_scope(&symbol.scope)
        {
            return true;
        }
        false
    }) && kind.is_none_or(|k| symbol.kind.is_named(k))
}

/// The one definition that `raw` (`name`, `Type::name`, ...) names in `symbols`, or why there is
/// not exactly one.
pub fn find_one<'a>(symbols: &'a [Symbol], raw: &str) -> Result<&'a Symbol, String> {
    let selector = SymbolSelector::parse(raw);
    let hits: Vec<&Symbol> = symbols
        .iter()
        .filter(|s| s.name == selector.name && selector.matches_scope(&s.scope))
        .collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("no definition `{raw}`")),
        many => {
            let shown = many
                .iter()
                .take(5)
                .map(|s| format!("{} at line {}", s.kind, s.first))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "`{raw}` names {} definitions ({shown}); qualify it as `Type::{}`",
                many.len(),
                selector.name
            ))
        }
    }
}

/// Close symbol names for a query that did not match. Short names are ignored because they are
/// common and ambiguous; candidates are ranked by edit distance, then name.
pub fn suggest<'a>(symbols: impl IntoIterator<Item = &'a Symbol>, query: &str) -> Vec<String> {
    ranked_names(
        symbols.into_iter().map(|symbol| symbol.name.clone()),
        query,
        5,
        false,
    )
    .into_iter()
    .map(|(_, name)| name)
    .collect()
}

/// Longest definition, in lines, whose source [`attach_bodies`] inlines.
pub const MAX_BODY_LINES: usize = 50;

/// Give each listed definition its source below its row when it spans at most
/// [`MAX_BODY_LINES`] lines and no other listed symbol lies inside it: a listed child shows its
/// own body, so the parent would repeat it. `source` is the text `listed` was read from. Body
/// rows start with two spaces and the source line number, so they cannot be taken for symbol
/// rows. The definition's first line is the signature already shown, so the body starts below it.
pub fn attach_bodies(listed: &mut [Symbol], source: &str) {
    let lines: Vec<&str> = source.lines().collect();
    let bodies: Vec<Option<String>> = listed
        .iter()
        .enumerate()
        .map(|(at, symbol)| {
            let holds_next = listed
                .get(at + 1)
                .is_some_and(|next| next.first >= symbol.first && next.last <= symbol.last);
            if holds_next || symbol.last.saturating_sub(symbol.head) >= MAX_BODY_LINES {
                return None;
            }
            let rows: String = (symbol.head + 1..=symbol.last.min(lines.len()))
                .map(|number| format!("  {number}\t{}\n", lines[number - 1]))
                .collect();
            (!rows.is_empty()).then_some(rows)
        })
        .collect();
    for (symbol, body) in listed.iter_mut().zip(bodies) {
        symbol.body = body;
    }
}

/// `lines<tab>signature`, one row per symbol. `lines` is `A..B`, or `A` for a definition on one
/// line. A symbol whose parent definition is listed directly above it is indented two spaces
/// under that parent. Any other symbol inside a definition ends with `(in scope)`, the qualified
/// path (`east::Token`), so it keeps its place when a filter or a page leaves its parent out.
/// With `docs`, a further column holds the first documentation line when there is one.
pub fn render(symbols: &[Symbol], docs: bool) -> String {
    let mut out = String::new();
    // The listed definitions that enclose the current symbol, outermost first.
    let mut open: Vec<Open> = Vec::new();
    for s in symbols {
        while open.last().is_some_and(|top| !top.encloses(s)) {
            open.pop();
        }
        let parent_indent = open
            .last()
            .filter(|top| top.path == s.scope)
            .map(|top| top.indent);
        let indent = parent_indent.map_or(0, |indent| indent + 1);
        out.push_str(&lines_label(s.first, s.last));
        out.push('\t');
        out.push_str(&"  ".repeat(indent));
        out.push_str(&s.signature);
        if parent_indent.is_none() && !s.scope.is_empty() {
            out.push_str(&format!(" (in {})", s.scope));
        }
        if docs && !s.doc.is_empty() {
            out.push_str(&format!("\t{}", s.doc));
        }
        out.push('\n');
        if let Some(body) = &s.body {
            out.push_str(body);
        }
        open.push(Open {
            path: s.scope.entered(s.frame()),
            first: s.first,
            last: s.last,
            indent,
        });
    }
    out
}

/// A listed definition that the symbols after it may be nested under.
struct Open {
    /// The qualified path of the definition itself: its scope and its own frame.
    path: Lineage,
    first: usize,
    last: usize,
    indent: usize,
}

impl Open {
    /// Whether `symbol` is written inside this definition.
    fn encloses(&self, symbol: &Symbol) -> bool {
        symbol.scope.starts_with(&self.path)
            && self.first <= symbol.first
            && symbol.last <= self.last
    }
}

/// `A..B`, or `A` when the definition is on one line.
fn lines_label(first: usize, last: usize) -> String {
    if first == last {
        first.to_string()
    } else {
        format!("{first}..{last}")
    }
}

fn walk(
    node: Node<'_>,
    scope: &mut Lineage,
    syntax: &dyn Syntax,
    source: &str,
    out: &mut Vec<Symbol>,
) {
    let mut entered = false;
    if let Some(def) = syntax.definition(node, source) {
        let (first, last) = unit_span(syntax, node, source);
        out.push(Symbol {
            kind: def.kind,
            name: def.name.clone(),
            trait_name: def.trait_name.clone(),
            test: def.test,
            scope: scope.clone(),
            first,
            last,
            head: node.start_position().row + 1,
            signature: header(node, source),
            doc: syntax.doc_line(node, source).unwrap_or_default(),
            body: None,
        });
        scope.push(Frame::of(&def));
        entered = true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, scope, syntax, source, out);
    }
    if entered {
        scope.pop();
    }
}
