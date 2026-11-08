//! Import statements: the names each one binds and the package it comes from.
//!
//! The call graph reads them per file to place a call that no indexed function answers to.
//! Only the [`Syntax`] trait is used.

use tree_sitter::Node;

use crate::language::{Import, Syntax};

/// Every import below `root`, in source order.
pub fn collect(root: Node<'_>, syntax: &dyn Syntax, source: &str) -> Vec<Import> {
    let mut out = Vec::new();
    visit(root, syntax, source, &mut out);
    out
}

fn visit(node: Node<'_>, syntax: &dyn Syntax, source: &str, out: &mut Vec<Import>) {
    out.extend(syntax.imports(node, source));
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(child, syntax, source, out);
    }
}

/// The import that gives `name` to the file: the one that binds it, else one whose last segment
/// is `name` under another local name (`use a::b as c` makes `b` mean `c`, and the call graph
/// reports the original).
pub fn bound<'a>(imports: &'a [Import], name: &str) -> Option<&'a Import> {
    imports
        .iter()
        .find(|import| import.local != "*" && import.local == name)
        .or_else(|| {
            imports.iter().find(|import| {
                import.local != "*" && import.path.last().is_some_and(|last| last == name)
            })
        })
}

/// Whether some import brings in every name of a module (`use a::*`, `from a import *`), so a
/// name that no other import binds may come from there.
pub fn has_wildcard(imports: &[Import]) -> bool {
    imports.iter().any(|import| import.local == "*")
}
