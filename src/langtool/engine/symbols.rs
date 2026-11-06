//! Symbol listing: the definitions in a file with their line span, parent scope and signature.
//!
//! Only the [`Syntax`] trait is used. Reading files and paginating results belong to the tool
//! handler.

use tree_sitter::Node;

use super::name_match::ranked_names;
use super::syntax::{Lineage, SymbolSelector, Syntax, header, parse, unit_span};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub kind: &'static str,
    pub name: String,
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
        symbol
            .name
            .to_lowercase()
            .contains(&selector.name.to_lowercase())
            && selector.matches_scope(&symbol.scope)
    }) && kind.is_none_or(|k| symbol.kind.eq_ignore_ascii_case(k))
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
        open.push(Open {
            path: s.scope.entered(s.kind, &s.name),
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
            scope: scope.clone(),
            first,
            last,
            head: node.start_position().row + 1,
            signature: header(node, source),
            doc: syntax.doc_line(node, source).unwrap_or_default(),
        });
        scope.push(def.kind, &def.name);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langtool::engine::syntax::python::PythonSyntax;
    use crate::langtool::engine::syntax::rust::RustSyntax;

    const RUST: &str = "struct P {\n    x: i32,\n}\n\nimpl P {\n    fn new() -> Self {\n        P { x: 0 }\n    }\n}\n";
    const PYTHON: &str = "LIMIT = 3\n\nclass A:\n    def f(self):\n        pass\n";

    #[test]
    fn lists_rust_symbols_with_scope() {
        let out = render(&symbols(&RustSyntax, RUST).unwrap(), false);
        assert_eq!(
            out,
            "1..3\tstruct P\n2\t  x: i32\n5..9\timpl P\n6..8\t  fn new() -> Self\n"
        );
    }

    #[test]
    fn nested_scopes_render_as_a_qualified_path() {
        let src = "mod east {\n    impl Token {\n        fn parse() {}\n    }\n}\n";
        let out = render(&symbols(&RustSyntax, src).unwrap(), false);
        assert_eq!(
            out,
            "1..5\tmod east\n2..4\t  impl Token\n3\t    fn parse()\n"
        );
    }

    #[test]
    fn lists_python_symbols_with_scope() {
        let out = render(&symbols(&PythonSyntax, PYTHON).unwrap(), false);
        assert_eq!(out, "1\tLIMIT = 3\n3..5\tclass A\n4..5\t  def f(self)\n");
    }

    #[test]
    fn children_nest_only_under_a_listed_parent() {
        let src = "mod east {\n    impl Token {\n        /// Parse it.\n        fn parse() {}\n    }\n}\nfn a() {} fn b() {}\nimpl S { fn p(&self) {} }\n";
        let all = symbols(&RustSyntax, src).unwrap();
        let listed = |keep: &dyn Fn(&Symbol) -> bool, docs: bool| {
            let kept: Vec<Symbol> = all.iter().filter(|s| keep(s)).cloned().collect();
            render(&kept, docs)
        };

        // Every level indents under its parent; siblings and one-line impls stay apart.
        assert_eq!(
            listed(&|_| true, false),
            "1..6\tmod east\n2..5\t  impl Token\n3..4\t    fn parse()\n7\tfn a()\n7\tfn b()\n8\timpl S\n8\t  fn p(&self)\n"
        );
        // Without the parents, each function says where it is.
        assert_eq!(
            listed(&|s| s.kind == "fn", false),
            "3..4\tfn parse() (in east::Token)\n7\tfn a()\n7\tfn b()\n8\tfn p(&self) (in S)\n"
        );
        // A listed ancestor that is not the direct parent does not hide the scope.
        assert_eq!(
            listed(&|s| s.kind != "impl", false),
            "1..6\tmod east\n3..4\tfn parse() (in east::Token)\n7\tfn a()\n7\tfn b()\n8\tfn p(&self) (in S)\n"
        );
        // The documentation column follows the scope suffix.
        assert_eq!(
            listed(&|s| s.name == "parse", true),
            "3..4\tfn parse() (in east::Token)\tParse it.\n"
        );
    }

    #[test]
    fn suggests_close_symbol_names_but_skips_short_names() {
        let all = symbols(&RustSyntax, "fn configure() {}\nfn collect() {}\n").unwrap();
        assert_eq!(suggest(&all, "configuer"), ["configure"]);
        assert!(suggest(&all, "co").is_empty());
        assert!(suggest(&all, "unrelated").is_empty());
    }

    #[test]
    fn signatures_keep_parameters_written_over_several_lines() {
        let rust = "pub fn long<T>(\n    a: i32,\n    b: T,\n) -> i32\nwhere\n    T: Copy,\n{\n    a\n}\n\nstruct S {\n    x: i32,\n}\n\nconst BIG: [u8; 2] = [\n    1,\n    2,\n];\n";
        let all = symbols(&RustSyntax, rust).unwrap();
        assert_eq!(
            all[0].signature,
            "pub fn long<T>(a: i32, b: T) -> i32 where T: Copy"
        );
        assert_eq!(all[1].signature, "struct S");
        assert_eq!(all[2].signature, "x: i32");
        assert_eq!(all[3].signature, "const BIG: [u8; 2] = [");
        let py = "def f(\n    a,\n    b,\n):\n    pass\n";
        assert_eq!(
            symbols(&PythonSyntax, py).unwrap()[0].signature,
            "def f(a, b)"
        );
        let long = format!("fn f({}) {{}}\n", "a: i32, ".repeat(60));
        let sig = &symbols(&RustSyntax, &long).unwrap()[0].signature;
        assert_eq!(sig.chars().count(), 241);
        assert!(sig.ends_with('…'));
    }

    #[test]
    fn filters_by_name_and_kind() {
        let all = symbols(&RustSyntax, RUST).unwrap();
        let by_name: Vec<_> = all
            .iter()
            .filter(|s| matches(s, Some("NEW"), None))
            .collect();
        assert_eq!(by_name.len(), 1);
        let by_kind: Vec<_> = all
            .iter()
            .filter(|s| matches(s, None, Some("impl")))
            .collect();
        assert_eq!(by_kind.len(), 1);
    }

    #[test]
    fn qualified_queries_match_all_enclosing_definitions() {
        let src = "mod east {\n    impl Token {\n        fn parse() {}\n    }\n}\nmod west {\n    impl Token {\n        fn parse() {}\n    }\n}\nfn parse() {}\n";
        let all = symbols(&RustSyntax, src).unwrap();
        let count = |q: &str| {
            all.iter()
                .filter(|s| matches(s, Some(q), Some("fn")))
                .count()
        };
        assert_eq!(count("parse"), 3);
        assert_eq!(count("Token::parse"), 2);
        assert_eq!(count("east::Token::parse"), 1);
        assert_eq!(count("west/parse"), 1);
        assert_eq!(count("west.east.parse"), 0);
    }
}
