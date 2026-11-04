//! Symbol listing: the definitions in a file with their line span, parent scope and signature.
//!
//! Only the [`Syntax`] trait is used. Reading files and paginating results belong to the tool
//! handler.

use tree_sitter::Node;

use super::syntax::{Syntax, header, line_span, parse};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub kind: &'static str,
    pub name: String,
    /// The enclosing definition as `kind name`, empty at the top level.
    pub scope: String,
    /// 1-based first and last line.
    pub first: usize,
    pub last: usize,
    /// First line of the definition as written, without a trailing `{` or `:`.
    pub signature: String,
}

/// All definitions in `source`, in source order. `None` when it cannot be parsed.
pub fn symbols(syntax: &dyn Syntax, source: &str) -> Option<Vec<Symbol>> {
    let tree = parse(syntax, source)?;
    let mut out = Vec::new();
    walk(tree.root_node(), "", syntax, source, &mut out);
    Some(out)
}

/// Keep symbols whose name contains `query` (case-insensitive) and whose kind equals `kind`.
pub fn matches(symbol: &Symbol, query: Option<&str>, kind: Option<&str>) -> bool {
    query.is_none_or(|q| symbol.name.to_lowercase().contains(&q.to_lowercase()))
        && kind.is_none_or(|k| symbol.kind.eq_ignore_ascii_case(k))
}

/// `A..B<tab>kind name (in scope)<tab>signature`, one line per symbol.
pub fn render(symbols: &[Symbol]) -> String {
    let mut out = String::new();
    for s in symbols {
        out.push_str(&format!("{}..{}\t{} {}", s.first, s.last, s.kind, s.name));
        if !s.scope.is_empty() {
            out.push_str(&format!(" (in {})", s.scope));
        }
        out.push_str(&format!("\t{}\n", s.signature));
    }
    out
}

fn walk(node: Node<'_>, scope: &str, syntax: &dyn Syntax, source: &str, out: &mut Vec<Symbol>) {
    let mut inner = scope.to_string();
    if let Some(def) = syntax.definition(node, source) {
        let (first, last) = line_span(node);
        out.push(Symbol {
            kind: def.kind,
            name: def.name.clone(),
            scope: scope.to_string(),
            first,
            last,
            signature: header(node, source),
        });
        inner = format!("{} {}", def.kind, def.name);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, &inner, syntax, source, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::syntax::python::PythonSyntax;
    use crate::lang::syntax::rust::RustSyntax;

    const RUST: &str = "struct P {\n    x: i32,\n}\n\nimpl P {\n    fn new() -> Self {\n        P { x: 0 }\n    }\n}\n";
    const PYTHON: &str = "LIMIT = 3\n\nclass A:\n    def f(self):\n        pass\n";

    #[test]
    fn lists_rust_symbols_with_scope() {
        let out = render(&symbols(&RustSyntax, RUST).unwrap());
        assert_eq!(
            out,
            "1..3\tstruct P\tstruct P\n5..9\timpl P\timpl P\n6..8\tfn new (in impl P)\tfn new() -> Self\n"
        );
    }

    #[test]
    fn lists_python_symbols_with_scope() {
        let out = render(&symbols(&PythonSyntax, PYTHON).unwrap());
        assert_eq!(
            out,
            "1..1\tvar LIMIT\tLIMIT = 3\n3..5\tclass A\tclass A\n4..5\tdef f (in class A)\tdef f(self)\n"
        );
    }

    #[test]
    fn filters_by_name_and_kind() {
        let all = symbols(&RustSyntax, RUST).unwrap();
        let by_name: Vec<_> = all.iter().filter(|s| matches(s, Some("NEW"), None)).collect();
        assert_eq!(by_name.len(), 1);
        let by_kind: Vec<_> = all.iter().filter(|s| matches(s, None, Some("impl"))).collect();
        assert_eq!(by_kind.len(), 1);
    }
}
