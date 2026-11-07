//! Outgoing dependencies: the names a definition uses.
//!
//! The counterpart of [`references`](super::references). Instead of the places that use a name,
//! it lists the names that the signature and body of a definition use, with their role and the
//! lines that use them. Names are not resolved here, only matched through import aliases; the
//! tool matches them to the project's definitions by name. Only the [`Syntax`] trait is used.

use std::collections::BTreeSet;

use tree_sitter::Node;

use super::aliases::{aliases, resolve};
use super::references::{Role, role};
use super::symbols::{self, Symbol};
use super::syntax::{SymbolSelector, Syntax, node_text, parse};

/// A name used inside a definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use {
    /// The name as it is defined: the original behind an import alias.
    pub name: String,
    /// The alias written in the code when it differs from `name`.
    pub alias: Option<String>,
    /// `call`, `type` or `use`, as in the reference roles.
    pub role: Role,
    /// 1-based lines of the occurrences, ascending.
    pub lines: Vec<usize>,
}

/// A definition and the names it uses, in order of first use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    pub symbol: Symbol,
    pub uses: Vec<Use>,
}

/// What every definition that `selector` names in `source` uses, in source order: the names in
/// its signature and body, except the ones it defines itself (nested definitions, Python
/// assignments, its own name), imports and fields. `None` when the source cannot be parsed.
pub fn dependencies(
    syntax: &dyn Syntax,
    source: &str,
    selector: &SymbolSelector<'_>,
) -> Option<Vec<Body>> {
    let wanted: Vec<Symbol> = symbols::symbols(syntax, source)?
        .into_iter()
        .filter(|s| s.name == selector.name && selector.matches_scope(&s.scope))
        .collect();
    let (Some(first), Some(last)) = (
        wanted.iter().map(|s| s.head).min(),
        wanted.iter().map(|s| s.last).max(),
    ) else {
        return Some(Vec::new());
    };
    let tree = parse(syntax, source)?;
    let imports = aliases(syntax, source)?;
    let mut found = Vec::new();
    collect(tree.root_node(), syntax, source, (first, last), &mut found);
    Some(
        wanted
            .into_iter()
            .map(|symbol| {
                let within = |line: &usize| (symbol.head..=symbol.last).contains(line);
                let defined: BTreeSet<&str> = found
                    .iter()
                    .filter(|(line, _, role)| within(line) && *role == Role::Def)
                    .map(|(_, text, _)| text.as_str())
                    .collect();
                let mut uses: Vec<Use> = Vec::new();
                for (line, text, role) in found.iter().filter(|(line, _, _)| within(line)) {
                    if matches!(*role, Role::Def | Role::Import | Role::Field)
                        || defined.contains(text.as_str())
                    {
                        continue;
                    }
                    let name = resolve(&imports, text);
                    match uses
                        .iter_mut()
                        .find(|used| used.name == name && used.role == *role)
                    {
                        Some(used) => {
                            if used.lines.last() != Some(line) {
                                used.lines.push(*line);
                            }
                        }
                        None => uses.push(Use {
                            name: name.to_string(),
                            alias: (name != text.as_str()).then(|| text.clone()),
                            role: *role,
                            lines: vec![*line],
                        }),
                    }
                }
                Body { symbol, uses }
            })
            .collect(),
    )
}

/// Every identifier on the lines `rows` (first, last), as (line, text, role), in source order.
fn collect(
    node: Node<'_>,
    syntax: &dyn Syntax,
    source: &str,
    rows: (usize, usize),
    out: &mut Vec<(usize, String, Role)>,
) {
    let (start, end) = (node.start_position().row + 1, node.end_position().row + 1);
    if end < rows.0 || start > rows.1 || syntax.text_kinds().contains(&node.kind()) {
        return;
    }
    if syntax.identifier_kinds().contains(&node.kind()) {
        out.push((
            start,
            node_text(node, source).to_string(),
            role(node, syntax, source),
        ));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, syntax, source, rows, out);
    }
}
