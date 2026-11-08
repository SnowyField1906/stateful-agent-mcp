//! Incoming references: where the requested names are defined and used. One scan answers every
//! name, and a name without references gets a hint with the closest identifiers of that scan.

use std::cell::Cell;
use std::collections::BTreeSet;

use super::index::is_test_path;
use super::name_match::ranked_names;
use super::references::{self, Reference};
use super::scan::{ScanTarget, scan};
use super::selector::SymbolSelector;
use crate::common::{AgentError, PageParams, Result, ServerConfig, Workspace};

/// The names to look up, where, and how much of the result to show.
pub struct Incoming<'a> {
    /// Names, or qualified selectors such as `Type::name`.
    pub symbols: &'a [&'a str],
    pub target: ScanTarget<'a>,
    /// Lower-cased text that the enclosing scope path of a reference must contain.
    pub scope: Option<String>,
    pub include_tests: bool,
    pub include_scope: bool,
    pub page: &'a PageParams,
}

/// A reference found for the requested symbol at index `symbol`.
struct Hit {
    symbol: usize,
    reference: Reference,
}

/// The references to every requested name, one page of them, with the hints that follow.
pub fn find(
    workspace: Option<&dyn Workspace>,
    cfg: &ServerConfig,
    query: &Incoming<'_>,
) -> Result<String> {
    let symbols = query.symbols;
    if symbols.is_empty() {
        return Err(AgentError::invalid("symbol", "must not be empty"));
    }
    let selectors: Vec<SymbolSelector<'_>> = symbols
        .iter()
        .map(|symbol| SymbolSelector::parse(symbol))
        .collect();
    if selectors.iter().any(|selector| selector.name.is_empty()) {
        return Err(AgentError::invalid("symbol", "must name a symbol"));
    }
    let include_tests = query.include_tests;
    let within = query.scope.as_deref();
    let hidden = Cell::new(0usize);
    let mut counts = vec![0usize; symbols.len()];
    let found = scan(
        workspace,
        &query.target,
        |_, syntax, text, label| -> Option<Vec<Hit>> {
            let mut hits = Vec::new();
            for (symbol, selector) in selectors.iter().enumerate() {
                let mut refs = references::references(syntax, text, selector.name)?;
                references::retain_qualified(&mut refs, selector);
                if let Some(within) = within {
                    refs.retain(|reference| {
                        reference.scope.to_string().to_lowercase().contains(within)
                    });
                }
                if !include_tests {
                    let all = refs.len();
                    if is_test_path(label) {
                        refs.clear();
                    } else {
                        refs.retain(|r| !r.in_test);
                    }
                    hidden.set(hidden.get() + all - refs.len());
                }
                counts[symbol] += refs.len();
                hits.extend(refs.into_iter().map(|reference| Hit { symbol, reference }));
            }
            Some(hits)
        },
    )?;
    let none = match symbols {
        [one] => format!("no references to `{one}`"),
        many => format!(
            "no references to any of {}",
            many.iter()
                .map(|symbol| format!("`{symbol}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let mut out = found.render(cfg, query.page, &none, |group| {
        render_hits(group, symbols, query.include_scope)
    })?;
    if hidden.get() > 0 {
        out.push_str(&format!(
            "({} references in tests hidden; set include_tests)\n",
            hidden.get()
        ));
    }
    let missing: Vec<usize> = (0..symbols.len()).filter(|&at| counts[at] == 0).collect();
    if symbols.len() > 1 && !out.starts_with("no references") {
        for &at in &missing {
            out.push_str(&format!("no references to `{}`\n", symbols[at]));
        }
    }
    if hidden.get() > 0 || missing.is_empty() {
        return Ok(out);
    }
    let names: Vec<&str> = missing.iter().map(|&at| selectors[at].name).collect();
    let similar = similar_names(workspace, &query.target, include_tests, &names)?;
    for (&at, near) in missing.iter().zip(similar) {
        if near.is_empty() {
            continue;
        }
        let near = near.join(", ");
        if symbols.len() > 1 {
            out.push_str(&format!(
                "hint: similar names for `{}`: {near}\n",
                symbols[at]
            ));
        } else {
            out.push_str(&format!("hint: similar names: {near}\n"));
        }
    }
    Ok(out)
}

/// One file's page of hits, each run of one symbol's references headed by `# name` when several
/// symbols were requested.
fn render_hits(group: &[Hit], symbols: &[&str], include_scope: bool) -> String {
    let mut out = String::new();
    for run in group.chunk_by(|a, b| a.symbol == b.symbol) {
        if symbols.len() > 1 {
            out.push_str(&format!("# {}\n", symbols[run[0].symbol]));
        }
        let refs = run.iter().map(|hit| &hit.reference);
        out.push_str(&if include_scope {
            references::render_with_scope(refs)
        } else {
            references::render(refs)
        });
    }
    out
}

/// Identifiers near each of `names`, from one scan, for the hint on a name without references.
fn similar_names(
    workspace: Option<&dyn Workspace>,
    target: &ScanTarget<'_>,
    include_tests: bool,
    names: &[&str],
) -> Result<Vec<Vec<String>>> {
    let scanned = scan(workspace, target, |_, syntax, text, label| {
        if !include_tests && is_test_path(label) {
            Some(Vec::new())
        } else {
            references::identifier_names(syntax, text, include_tests)
        }
    })?;
    let (files, _) = scanned.into_parts();
    let candidates: BTreeSet<String> = files.into_iter().flat_map(|(_, found)| found).collect();
    let candidates: Vec<String> = candidates.into_iter().collect();
    Ok(names
        .iter()
        .map(|name| {
            ranked_names(candidates.clone(), name, 5, false)
                .into_iter()
                .map(|(_, near)| near)
                .collect()
        })
        .collect())
}
