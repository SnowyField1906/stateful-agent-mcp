//! Import aliases (`use a::b as c`, `import numpy as np`, `from m import f as g`).
//!
//! Aliases are read per file and matched by name only: no module path or crate resolution. The
//! reference lookup and the call graph use them so that a renamed import still counts as a use
//! of the original name. Only the [`Syntax`] trait is used.

use tree_sitter::Node;

use super::syntax::{Alias, Syntax, parse};

/// All import aliases declared in `source`, in source order. `None` when it cannot be parsed.
pub fn aliases(syntax: &dyn Syntax, source: &str) -> Option<Vec<Alias>> {
    let tree = parse(syntax, source)?;
    let mut out = Vec::new();
    collect(tree.root_node(), syntax, source, &mut out);
    Some(out)
}

/// `symbol` followed by every alias the file gives it: the names that mean `symbol` there.
pub fn names_for(aliases: &[Alias], symbol: &str) -> Vec<String> {
    std::iter::once(symbol.to_string())
        .chain(
            aliases
                .iter()
                .filter(|a| a.original == symbol)
                .map(|a| a.alias.clone()),
        )
        .collect()
}

/// The original name behind `name` when the file imports it under that alias, else `name`.
pub fn resolve<'a>(aliases: &'a [Alias], name: &'a str) -> &'a str {
    aliases
        .iter()
        .find(|a| a.alias == name)
        .map_or(name, |a| a.original.as_str())
}

fn collect(node: Node<'_>, syntax: &dyn Syntax, source: &str, out: &mut Vec<Alias>) {
    if let Some(alias) = syntax.alias(node, source) {
        out.push(alias);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, syntax, source, out);
    }
}
