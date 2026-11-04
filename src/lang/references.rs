//! Reference lookup: where a name is defined and used, ignoring comments and string literals.
//!
//! Only the [`Syntax`] trait is used. Walking directories, reading files and paginating results
//! belong to the tool handler.

use tree_sitter::Node;

use super::aliases::{aliases, names_for};
use super::syntax::{Syntax, node_text, parse};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// 1-based line.
    pub line: usize,
    /// Whether this occurrence is the name of a definition rather than a use.
    pub definition: bool,
    /// The import alias written here instead of the symbol, when this occurrence uses one.
    pub alias: Option<String>,
    /// The source line, trimmed.
    pub text: String,
}

/// Every identifier in `source` whose text equals `symbol` or an import alias the file gives it,
/// in source order. `None` when the source cannot be parsed.
pub fn references(syntax: &dyn Syntax, source: &str, symbol: &str) -> Option<Vec<Reference>> {
    if symbol.is_empty() || !source.contains(symbol) {
        return Some(Vec::new());
    }
    let tree = parse(syntax, source)?;
    let names = names_for(&aliases(syntax, source)?, symbol);
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    collect(tree.root_node(), &names, syntax, source, &lines, &mut out);
    Some(out)
}

/// `line:text` for uses, `line:def:text` for definitions and `line:alias NAME:text` for uses
/// through an import alias, one per line.
pub fn render(refs: &[Reference]) -> String {
    let mut out = String::new();
    for r in refs {
        if r.definition {
            out.push_str(&format!("{}:def:{}\n", r.line, r.text));
        } else if let Some(alias) = &r.alias {
            out.push_str(&format!("{}:alias {}:{}\n", r.line, alias, r.text));
        } else {
            out.push_str(&format!("{}:{}\n", r.line, r.text));
        }
    }
    out
}

/// `names[0]` is the symbol itself; the rest are its aliases in this file.
fn collect(
    node: Node<'_>,
    names: &[String],
    syntax: &dyn Syntax,
    source: &str,
    lines: &[&str],
    out: &mut Vec<Reference>,
) {
    if syntax.text_kinds().contains(&node.kind()) {
        return;
    }
    if syntax.identifier_kinds().contains(&node.kind()) {
        let text = node_text(node, source);
        if let Some(i) = names.iter().position(|n| n == text) {
            let row = node.start_position().row;
            out.push(Reference {
                line: row + 1,
                definition: is_definition_name(node, syntax, source),
                alias: (i > 0).then(|| text.to_string()),
                text: lines.get(row).map(|l| l.trim().to_string()).unwrap_or_default(),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, names, syntax, source, lines, out);
    }
}

/// Whether `node` is the name a definition introduces (`name` field, or `left` for a Python
/// assignment).
fn is_definition_name(node: Node<'_>, syntax: &dyn Syntax, source: &str) -> bool {
    let Some(parent) = node.parent() else { return false };
    if syntax.definition(parent, source).is_none() {
        return false;
    }
    ["name", "left"]
        .into_iter()
        .any(|field| parent.child_by_field_name(field).is_some_and(|n| n.id() == node.id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::syntax::python::PythonSyntax;
    use crate::lang::syntax::rust::RustSyntax;

    const RUST: &str = "// helper in a comment\nfn helper() {}\n\nfn main() {\n    let s = \"helper\";\n    helper();\n}\n";
    const PYTHON: &str = "LIMIT = 3\n# LIMIT in a comment\n\ndef f():\n    return LIMIT + 1\n";

    #[test]
    fn finds_rust_definition_and_use_but_not_comment_or_string() {
        let refs = references(&RustSyntax, RUST, "helper").unwrap();
        assert_eq!(render(&refs), "2:def:fn helper() {}\n6:helper();\n");
    }

    #[test]
    fn finds_python_assignment_and_use() {
        let refs = references(&PythonSyntax, PYTHON, "LIMIT").unwrap();
        assert_eq!(render(&refs), "1:def:LIMIT = 3\n5:return LIMIT + 1\n");
    }

    #[test]
    fn finds_uses_through_a_rust_import_alias() {
        let src = "use a::helper as h;\nfn main() {\n    h();\n    helper();\n}\n";
        let refs = references(&RustSyntax, src, "helper").unwrap();
        assert_eq!(
            render(&refs),
            "1:use a::helper as h;\n1:alias h:use a::helper as h;\n3:alias h:h();\n4:helper();\n"
        );
    }

    #[test]
    fn finds_uses_through_a_python_import_alias() {
        let src = "from m import f as g\ng()\nf()\n";
        let refs = references(&PythonSyntax, src, "f").unwrap();
        assert_eq!(
            render(&refs),
            "1:from m import f as g\n1:alias g:from m import f as g\n2:alias g:g()\n3:f()\n"
        );
    }

    #[test]
    fn missing_symbol_gives_no_references() {
        assert!(references(&RustSyntax, RUST, "nothing").unwrap().is_empty());
        assert!(references(&RustSyntax, RUST, "").unwrap().is_empty());
    }
}
