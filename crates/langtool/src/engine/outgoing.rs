//! Outgoing references: what the named definitions use. One scan reads the definitions and also
//! gathers every definition of the project, which the used names are then matched to by name.

use std::collections::HashMap;

use super::dependencies::{self, Body};
use super::index::is_test_path;
use super::origins::Origins;
use super::outside::Outside;
use super::references::Role;
use super::scan::{ScanTarget, scan};
use super::selector::SymbolSelector;
use super::symbols::{self, Symbol};
use crate::common::fs::resolve::Scope;
use crate::common::{AgentError, PageParams, Result, ServerConfig, Workspace, root_of};
use crate::language::{CallKind, CallSite};

/// The definitions to read, where, and how much of the result to show.
pub struct Outgoing<'a> {
    /// Names, or qualified selectors such as `Type::name`.
    pub from: &'a [&'a str],
    pub target: ScanTarget<'a>,
    /// Lower-cased text that the scope path of a definition must contain.
    pub scope: Option<String>,
    pub include_tests: bool,
    pub page: &'a PageParams,
}

/// A definition's used names together with the file it is in.
struct Located {
    file: String,
    body: Body,
}

/// What each named definition uses, one page of it, with the hints that follow.
pub fn find(
    workspace: Option<&dyn Workspace>,
    cfg: &ServerConfig,
    query: &Outgoing<'_>,
) -> Result<String> {
    let from = query.from;
    let selectors: Vec<SymbolSelector<'_>> = from
        .iter()
        .map(|name| SymbolSelector::parse(name))
        .collect();
    if selectors.iter().any(|selector| selector.name.is_empty()) {
        return Err(AgentError::invalid("from_symbol", "must name a symbol"));
    }
    let include_tests = query.include_tests;
    let within = query.scope.as_deref();
    let mut defs: Vec<(String, Symbol)> = Vec::new();
    let mut matched = vec![false; selectors.len()];
    let found = scan(
        workspace,
        &query.target,
        |_, syntax, text, label| -> Option<Vec<Located>> {
            let in_tests = is_test_path(label);
            let mut bodies = Vec::new();
            for (at, selector) in selectors.iter().enumerate() {
                let mut named = dependencies::dependencies(syntax, text, selector)?;
                if let Some(within) = within {
                    named.retain(|body| {
                        body.symbol
                            .scope
                            .to_string()
                            .to_lowercase()
                            .contains(within)
                    });
                }
                if !include_tests {
                    named.retain(|body| !in_tests && !body.symbol.scope.in_test());
                }
                matched[at] |= !named.is_empty();
                bodies.extend(named.into_iter().map(|body| Located {
                    file: label.to_string(),
                    body,
                }));
            }
            if include_tests || !in_tests {
                defs.extend(
                    symbols::symbols(syntax, text)?
                        .into_iter()
                        .filter(|symbol| {
                            !symbol.kind.on_request && (include_tests || !symbol.scope.in_test())
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
    let resolved = Scope::new(root_of(workspace), query.target.path, "implicit_path")?;
    let origins = Origins::new(&resolved.root, &[]);
    let mut out = found.render(cfg, query.page, &none, |group| {
        render_bodies(group, &index, &origins)
    })?;
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
/// (as alias)<tab>@lines<tab>definitions`, and one `external` row that summarizes the called or
/// type names it does not define by where they come from, as `langtool_view_callgraph` does.
fn render_bodies(
    group: &[Located],
    index: &HashMap<&str, Vec<&(String, Symbol)>>,
    origins: &Origins<'_>,
) -> String {
    let mut out = String::new();
    for Located { file, body } in group {
        out.push_str(&symbols::render(std::slice::from_ref(&body.symbol), false));
        let mut outside = Outside::default();
        let mut placed: Vec<&str> = Vec::new();
        for used in &body.uses {
            let Some(defs) = index.get(used.name.as_str()) else {
                if matches!(used.role, Role::Call | Role::Type)
                    && !placed.contains(&used.name.as_str())
                {
                    placed.push(&used.name);
                    let site = used.site.clone().unwrap_or_else(|| CallSite {
                        name: used.name.clone(),
                        kind: CallKind::Any,
                        root: None,
                    });
                    let verdict = origins.place(&site, file, &body.imports);
                    outside.add(&used.name, &verdict.origin);
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
        if !outside.is_empty() {
            out.push_str(&format!("  external\t{}\n", outside.render()));
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
