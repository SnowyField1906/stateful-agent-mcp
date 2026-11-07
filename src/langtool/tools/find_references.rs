//! `langtool_find_references`: where a name is defined and used, for languages with a
//! [`Syntax`](crate::langtool::Syntax). Output, grouped by file, one `line:role:text` per reference:
//! ```text
//! src/a.rs
//! 1:def:fn helper() {}
//! 4:call:helper();
//! ```

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::common::params::{Names, PageParams};
use crate::common::{AgentError, Result, ServerConfig};
use crate::langtool::engine::callgraph::is_test_path;
use crate::langtool::engine::dependencies::{self, Body};
use crate::langtool::engine::name_match::ranked_names;
use crate::langtool::engine::references::{self, Reference, Role};
use crate::langtool::engine::scan::{ScanTarget, scan};
use crate::langtool::engine::symbols::{self, Symbol};
use crate::langtool::engine::syntax::SymbolSelector;
use crate::workflow::Project;

pub const DESCRIPTION: &str = "Find name-based references in code with Tree-sitter, ignoring comments and strings. `symbol` is one name or a list, all searched in one scan; with a list, each file's rows are grouped under `# name` lines. A qualified `symbol` (`Type::name`) keeps references inside `Type` and those whose line names it. Filter by lexical scope substring (`scope`); `include_scope` adds scope paths to results. Output per file: `line:role:text`, with role `def`, `import`, `call`, `type`, `field` or `use`; `role(as h)` marks a use through the import alias `h`. Matching remains name-based; lexical scope narrows candidates but does not resolve same-named bindings. References in test code are hidden unless `include_tests`, and the count of hidden ones is reported. Give `from_symbol` (a name or list) instead of `symbol` for the opposite direction: the definition's row, then one row `role<tab>name<tab>@lines<tab>definitions` per name its signature and body use, where definitions are the project's (`file:line kind Qualified::name`), and an `external` row for called or type names that no indexed file defines. Names the definition defines itself, imports, fields and uses that match no project definition (usually locals) are left out.";

/// Where a name is defined and used in code with a registered Tree-sitter grammar.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FindReferencesParams {
    /// Name to look up: a function, type, variable, field or module name, or a list of them,
    /// all searched in one scan and shown under `# name` lines. `Type::name`, `Type.name` or
    /// `Type/name` keeps the references inside `Type` and those that name it.
    #[serde(default)]
    #[schemars(with = "Vec<String>")]
    pub symbol: Names,
    /// Instead of the uses of `symbol`, what the definition(s) named here use: one row per name
    /// in the signature and body, with the lines and the project definitions of that name. Give
    /// this or `symbol`. A list is answered from one scan.
    #[serde(default)]
    #[schemars(with = "Option<Vec<String>>")]
    pub from_symbol: Option<Names>,
    /// File or directory, relative to the project root or absolute. Default `.`.
    #[serde(default)]
    pub implicit_path: Option<String>,
    /// Include references in test code (test files, `mod tests`, `Test*` classes). Default false.
    #[serde(default)]
    pub include_tests: Option<bool>,
    /// Only references whose enclosing scope path contains this text (case-insensitive).
    #[serde(default)]
    pub scope: Option<String>,
    /// Add each reference's lexical scope path to its output row. Default false.
    #[serde(default)]
    pub include_scope: Option<bool>,
    /// Include dotfiles and gitignored files.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

/// A reference found for the requested symbol at index `symbol`.
struct Hit {
    symbol: usize,
    reference: Reference,
}

pub fn find_references(
    project: Option<&Project>,
    cfg: &ServerConfig,
    p: FindReferencesParams,
) -> Result<String> {
    let symbols = p.symbol.cleaned();
    let from = p
        .from_symbol
        .as_ref()
        .map(Names::cleaned)
        .unwrap_or_default();
    if !from.is_empty() {
        if !symbols.is_empty() {
            return Err(AgentError::invalid(
                "from_symbol",
                "give `symbol` or `from_symbol`, not both",
            ));
        }
        return outgoing(project, cfg, &p, &from);
    }
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
    let include_tests = p.include_tests.unwrap_or(false);
    let scope = p
        .scope
        .as_deref()
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(str::to_lowercase);
    let include_scope = p.include_scope.unwrap_or(false);
    let hidden = Cell::new(0usize);
    let mut counts = vec![0usize; symbols.len()];
    let target = ScanTarget {
        path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };
    let found = scan(
        project,
        &target,
        |_, syntax, text, label| -> Option<Vec<Hit>> {
            let mut hits = Vec::new();
            for (symbol, selector) in selectors.iter().enumerate() {
                let mut refs = references::references(syntax, text, selector.name)?;
                references::retain_qualified(&mut refs, selector);
                if let Some(scope) = &scope {
                    refs.retain(|reference| {
                        reference.scope.to_string().to_lowercase().contains(scope)
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
    let none = match symbols.as_slice() {
        [one] => format!("no references to `{one}`"),
        many => format!(
            "no references to any of {}",
            many.iter()
                .map(|symbol| format!("`{symbol}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let mut out = found.render(cfg, &p.page, &none, |group| {
        render_hits(group, &symbols, include_scope)
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
    let similar = similar_names(project, &target, include_tests, &names)?;
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
    project: Option<&Project>,
    target: &ScanTarget<'_>,
    include_tests: bool,
    names: &[&str],
) -> Result<Vec<Vec<String>>> {
    let scanned = scan(project, target, |_, syntax, text, label| {
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

/// `from_symbol`: what the named definitions use. One scan reads the definitions and also
/// gathers every definition of the project, which the used names are then matched to by name.
fn outgoing(
    project: Option<&Project>,
    cfg: &ServerConfig,
    p: &FindReferencesParams,
    from: &[&str],
) -> Result<String> {
    let selectors: Vec<SymbolSelector<'_>> = from
        .iter()
        .map(|name| SymbolSelector::parse(name))
        .collect();
    if selectors.iter().any(|selector| selector.name.is_empty()) {
        return Err(AgentError::invalid("from_symbol", "must name a symbol"));
    }
    let include_tests = p.include_tests.unwrap_or(false);
    let scope = p
        .scope
        .as_deref()
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(str::to_lowercase);
    let target = ScanTarget {
        path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };
    let mut defs: Vec<(String, Symbol)> = Vec::new();
    let mut matched = vec![false; selectors.len()];
    let found = scan(
        project,
        &target,
        |_, syntax, text, label| -> Option<Vec<Body>> {
            let in_tests = is_test_path(label);
            let mut bodies = Vec::new();
            for (at, selector) in selectors.iter().enumerate() {
                let mut named = dependencies::dependencies(syntax, text, selector)?;
                if let Some(scope) = &scope {
                    named.retain(|body| {
                        body.symbol.scope.to_string().to_lowercase().contains(scope)
                    });
                }
                if !include_tests {
                    named.retain(|body| !in_tests && !body.symbol.scope.in_test());
                }
                matched[at] |= !named.is_empty();
                bodies.extend(named);
            }
            if include_tests || !in_tests {
                defs.extend(
                    symbols::symbols(syntax, text)?
                        .into_iter()
                        .filter(|symbol| {
                            !syntax.extra_kinds().contains(&symbol.kind)
                                && (include_tests || !symbol.scope.in_test())
                        })
                        .map(|symbol| (label.to_string(), symbol)),
                );
            }
            Some(bodies)
        },
    )?;
    let mut index: HashMap<&str, Vec<&(String, Symbol)>> = HashMap::new();
    for def in &defs {
        index.entry(def.1.name.as_str()).or_default().push(def);
    }
    let none = format!(
        "no definition {}",
        from.iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut out = found.render(cfg, &p.page, &none, |group| render_bodies(group, &index))?;
    for (at, name) in from.iter().enumerate().filter(|(at, _)| !matched[*at]) {
        if from.len() > 1 && !out.starts_with("no definition") {
            out.push_str(&format!("no definition `{name}`\n"));
        }
        let near = symbols::suggest(defs.iter().map(|(_, symbol)| symbol), selectors[at].name);
        if near.is_empty() {
            continue;
        }
        let near = near.join(", ");
        if from.len() > 1 {
            out.push_str(&format!(
                "hint: similar symbol names for `{name}`: {near}\n"
            ));
        } else {
            out.push_str(&format!("hint: similar symbol names: {near}\n"));
        }
    }
    Ok(out)
}

/// Each definition's row, then one row per used name that the project defines, `role<tab>name
/// (as alias)<tab>@lines<tab>definitions`, and one `external` row with the called or type names
/// that it does not.
fn render_bodies(group: &[Body], index: &HashMap<&str, Vec<&(String, Symbol)>>) -> String {
    let mut out = String::new();
    for body in group {
        out.push_str(&symbols::render(std::slice::from_ref(&body.symbol), false));
        let mut external: Vec<String> = Vec::new();
        for used in &body.uses {
            let Some(defs) = index.get(used.name.as_str()) else {
                if matches!(used.role, Role::Call | Role::Type) && !external.contains(&used.name) {
                    external.push(used.name.clone());
                }
                continue;
            };
            let alias = used
                .alias
                .as_ref()
                .map(|alias| format!(" (as {alias})"))
                .unwrap_or_default();
            let lines = capped(used.lines.iter().map(usize::to_string), 3, ",");
            let targets = capped(
                defs.iter().map(|(file, def)| {
                    let name = if def.scope.is_empty() {
                        def.name.clone()
                    } else {
                        format!("{}::{}", def.scope, def.name)
                    };
                    format!("{file}:{} {} {name}", def.head, def.kind)
                }),
                3,
                ", ",
            );
            out.push_str(&format!(
                "  {}\t{}{alias}\t@{lines}\t{targets}\n",
                used.role, used.name
            ));
        }
        if !external.is_empty() {
            out.push_str(&format!("  external\t{}\n", capped(external, 8, ", ")));
        }
    }
    out
}

/// The first `shown` items joined by `separator`, then ` +N` for the rest.
fn capped(items: impl IntoIterator<Item = String>, shown: usize, separator: &str) -> String {
    let items: Vec<String> = items.into_iter().collect();
    let mut out = items
        .iter()
        .take(shown)
        .cloned()
        .collect::<Vec<_>>()
        .join(separator);
    if items.len() > shown {
        out.push_str(&format!(" +{}", items.len() - shown));
    }
    out
}
