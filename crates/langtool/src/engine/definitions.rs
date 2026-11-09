//! Definition listing for `langtool_find_symbols`: the symbols of every scanned file that match
//! the requested names, scope and kinds, followed by the closest names for a query that matched
//! nothing.

use std::collections::BTreeMap;

use super::name_match::similarity_score;
use super::scan::{ScanTarget, scan};
use super::selector::SymbolSelector;
use super::symbols::{self, Symbol};
use crate::common::fs::resolve::Scope;
use crate::common::{AgentError, PageParams, Result, ServerConfig, Workspace, root_of};
use crate::language::{Kind, registry};

/// What to list, where, and how much of the result to show.
pub struct Listing<'a> {
    /// Names, or qualified selectors such as `Type::name`; every symbol when empty.
    pub queries: &'a [&'a str],
    /// Lower-cased text that the enclosing scope path of a symbol must contain.
    pub scope: Option<String>,
    /// Kinds to keep by language name, as the caller wrote them (names or aliases).
    pub kinds: Option<&'a BTreeMap<String, Vec<String>>>,
    pub target: ScanTarget<'a>,
    /// Add the first documentation line of each symbol.
    pub docs: bool,
    /// Print the source of each listed symbol.
    pub include_body: bool,
    pub page: &'a PageParams,
}

/// The name of the kind in `kinds` that `raw` names, also by its aliases (`function`, `module`).
fn canonical_kind(kinds: &[&'static Kind], raw: &str) -> Option<&'static str> {
    kinds
        .iter()
        .find(|kind| kind.is_named(raw))
        .map(|kind| kind.name)
}

/// The kinds to keep per language, checked against the grammars. `None` keeps each language's
/// default set.
fn kind_filter(
    raw: Option<&BTreeMap<String, Vec<String>>>,
) -> Result<Option<BTreeMap<String, Vec<String>>>> {
    let Some(raw) = raw.filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (lang, list) in raw {
        let key = lang.trim().to_ascii_lowercase();
        let Some(syntax) = registry::supported_languages()
            .find(|language| language.name == key)
            .and_then(registry::of)
        else {
            return Err(AgentError::invalid(
                "kinds",
                format!(
                    "unsupported language `{lang}`; supported: {}",
                    registry::supported_names()
                ),
            ));
        };
        let kinds = syntax.kinds();
        let valid: Vec<&str> = kinds.iter().map(|kind| kind.name).collect();
        let mut cleaned: Vec<String> = Vec::new();
        for raw in list.iter().map(|k| k.trim()).filter(|k| !k.is_empty()) {
            let kind = canonical_kind(kinds, raw).ok_or_else(|| {
                AgentError::invalid(
                    "kinds",
                    format!("unknown {key} kind `{raw}`; valid: {}", valid.join(", ")),
                )
            })?;
            cleaned.push(kind.to_string());
        }
        map.entry(key).or_default().extend(cleaned);
    }
    Ok(Some(map))
}

/// The symbols that match `query`, one page of them grouped by file, with a hint for every name
/// that matched nothing.
pub fn list(
    workspace: Option<&dyn Workspace>,
    cfg: &ServerConfig,
    query: &Listing<'_>,
) -> Result<String> {
    let queries = query.queries;
    let selectors: Vec<SymbolSelector<'_>> = queries
        .iter()
        .map(|query| SymbolSelector::parse(query))
        .collect();
    let kinds_filter = kind_filter(query.kinds)?;
    let scope = query.scope.as_deref();
    let docs = query.docs;
    // Per query: whether a definition matched it, and the close names for when none did.
    let mut matched = vec![false; queries.len()];
    let mut suggestions = vec![BTreeMap::<String, usize>::new(); queries.len()];
    // Files below the project root are noted as seen when their bodies are shown; the labels of a
    // scan outside it are not project paths.
    let in_project = Scope::new(root_of(workspace), query.target.path, "implicit_path")?.in_project;
    let found = scan(workspace, &query.target, |lang, syntax, text, label| {
        let allowed = match &kinds_filter {
            Some(map) => match map.get(lang.name) {
                Some(list) if !list.is_empty() => Some(list),
                _ => return Some(Vec::new()),
            },
            None => None,
        };
        let all = symbols::symbols(syntax, text)?;
        let kind_allowed = |symbol: &Symbol| match allowed {
            Some(list) => list.iter().any(|k| symbols::matches(symbol, None, Some(k))),
            None => !symbol.kind.on_request,
        };
        let scope_allowed = |symbol: &Symbol| {
            scope.is_none_or(|scope| symbol.scope.to_string().to_lowercase().contains(scope))
        };
        for (selector, known) in selectors.iter().zip(suggestions.iter_mut()) {
            for symbol in all.iter().filter(|symbol| {
                kind_allowed(symbol)
                    && scope_allowed(symbol)
                    && selector.matches_scope(&symbol.scope)
            }) {
                if let Some(distance) = similarity_score(selector.name, &symbol.name) {
                    known
                        .entry(symbol.name.clone())
                        .and_modify(|best| *best = (*best).min(distance))
                        .or_insert(distance);
                }
            }
        }
        let mut listed: Vec<Symbol> = all
            .into_iter()
            .filter(|symbol| {
                if !kind_allowed(symbol) || !scope_allowed(symbol) {
                    return false;
                }
                if queries.is_empty() {
                    return true;
                }
                let mut any = false;
                for (&query, hit) in queries.iter().zip(matched.iter_mut()) {
                    if symbols::matches(symbol, Some(query), None) {
                        *hit = true;
                        any = true;
                    }
                }
                any
            })
            .collect();
        if query.include_body {
            symbols::attach_bodies(&mut listed, text);
            if in_project
                && listed.iter().any(|symbol| symbol.body.is_some())
                && let Some(workspace) = workspace
            {
                workspace.note_seen(label, text);
            }
        }
        Some(listed)
    })?;
    let mut out = found.render(cfg, query.page, "no symbols", |group| {
        symbols::render(group, docs)
    })?;
    for (at, name) in queries.iter().enumerate().filter(|(at, _)| !matched[*at]) {
        let mut near: Vec<_> = std::mem::take(&mut suggestions[at]).into_iter().collect();
        near.sort_by_key(|(name, distance)| (*distance, name.clone()));
        let near = near
            .into_iter()
            .take(5)
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&match (queries.len() > 1, near.is_empty()) {
            (false, true) => String::new(),
            (false, false) => format!("hint: similar symbol names: {near}\n"),
            (true, true) => format!("no symbols for `{name}`\n"),
            (true, false) => {
                format!("no symbols for `{name}`; similar symbol names: {near}\n")
            }
        });
    }
    Ok(out)
}
