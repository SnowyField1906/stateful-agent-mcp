//! Outgoing dependencies: the names a definition uses.
//!
//! The counterpart of [`references`](super::references). Instead of the places that use a name,
//! it lists the names that the signature and body of a definition use, with their role and the
//! lines that use them. Names are not resolved here, only matched through import aliases; the
//! tool matches them to the project's definitions by name. A call also keeps how it was written
//! (`serde_json::to_string`), which tells where a call with no project definition goes. Only the
//! [`Syntax`] trait is used.

use std::collections::BTreeSet;
use std::sync::Arc;

use tree_sitter::Node;

use super::aliases::{aliases, resolve};
use super::imports as file_imports;
use super::index::canonical_site;
use super::references::{Role, role};
use super::selector::SymbolSelector;
use super::symbols::{self, Symbol};
use crate::language::{CallSite, Import, Syntax, node_text, parse};

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
    /// For a call, how its first occurrence was written (`serde_json::to_string(..)`, `x.len()`).
    pub site: Option<CallSite>,
}

/// A definition and the names it uses, in order of first use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    pub symbol: Symbol,
    pub uses: Vec<Use>,
    /// The imports of the file, which tell where a used name that the project does not define
    /// comes from.
    pub imports: Arc<Vec<Import>>,
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
    let scanned_imports = Arc::new(file_imports::collect(tree.root_node(), syntax, source));
    let mut found = Vec::new();
    let mut sites = Vec::new();
    collect(
        tree.root_node(),
        syntax,
        source,
        (first, last),
        &mut found,
        &mut sites,
    );
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
                        None => {
                            let site = (*role == Role::Call)
                                .then(|| {
                                    sites
                                        .iter()
                                        .find(|(at, site)| within(at) && site.name == *text)
                                        .map(|(_, site)| {
                                            let mut site = site.clone();
                                            canonical_site(&mut site, &imports);
                                            site
                                        })
                                })
                                .flatten();
                            uses.push(Use {
                                name: name.to_string(),
                                alias: (name != text.as_str()).then(|| text.clone()),
                                role: *role,
                                lines: vec![*line],
                                site,
                            });
                        }
                    }
                }
                Body {
                    symbol,
                    uses,
                    imports: Arc::clone(&scanned_imports),
                }
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
    sites: &mut Vec<(usize, CallSite)>,
) {
    let (start, end) = (node.start_position().row + 1, node.end_position().row + 1);
    if end < rows.0 || start > rows.1 || syntax.text_kinds().contains(&node.kind()) {
        return;
    }
    if syntax.identifier(node).is_some() {
        out.push((
            start,
            node_text(node, source).to_string(),
            role(node, syntax, source),
        ));
    }
    if let Some(site) = syntax.call_site(node, source) {
        sites.push((start, site));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, syntax, source, rows, out, sites);
    }
}
