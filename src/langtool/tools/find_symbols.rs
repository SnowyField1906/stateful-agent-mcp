//! `langtool_find_symbols`: definitions in a file or below a directory, for languages with a
//! [`Syntax`](crate::langtool::Syntax). Output, grouped by file:
//! ```text
//! src/a.rs
//! 5..9\timpl P
//! 6..8\t  fn new() -> Self
//! ```

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};

use crate::common::params::{Names, PageParams};
use crate::common::{AgentError, Result, ServerConfig};
use crate::langtool::engine::name_match::similarity_score;
use crate::langtool::engine::scan::{ScanTarget, scan};
use crate::langtool::engine::symbols::{self, Symbol};
use crate::langtool::engine::syntax::{SymbolSelector, registry};
use crate::workflow::Project;

pub const DESCRIPTION: &str = "List definitions in files with a registered Tree-sitter grammar, in one file or below a directory. Each row is the source lines and the signature, indented under its parent definition, or ending with `(in Parent::Path)` when the parent is not listed; filter by `query` (one name or a list, each a name substring; `Type::name` also requires `Type` to enclose it), `scope` (enclosing-scope substring) or `kinds` (language-to-kind map; errors list allowed kinds). Language-specific nested items are opt-in; `docs` adds the first documentation line; `include_body` prints the source below the row of each listed definition of up to 50 lines that has no listed definition inside it, as rows `  N<tab>text`. Use the ranges with `langtool_read_file`.";

/// Definitions (functions, types, modules, ...) in a file or below a directory.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FindSymbolsParams {
    /// File or directory, relative to the project root or absolute. Default `.`.
    #[serde(default)]
    pub implicit_path: Option<String>,
    /// Only symbols whose name contains this text (case-insensitive); a list shows the symbols
    /// that match any of its entries. A qualified query such as `Store::put`, `Store.put` or
    /// `Store/put` also requires `Store` to enclose the symbol.
    #[serde(default)]
    #[schemars(with = "Option<Vec<String>>")]
    pub query: Option<Names>,
    /// Only symbols whose enclosing scope, written as a qualified path (`east::Token`), contains
    /// this text (case-insensitive).
    #[serde(default)]
    pub scope: Option<String>,
    /// Kinds to include by language (`{ [language]: [kinds...] }`), for example `{"rust": ["fn", "struct"]}`. Valid kinds depend on the registered grammar.
    #[serde(default, deserialize_with = "opt_kinds_map")]
    #[schemars(with = "Option<BTreeMap<String, Vec<String>>>")]
    pub kinds: Option<BTreeMap<String, Vec<String>>>,
    /// Include dotfiles and gitignored files.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
    /// Add the first documentation line of each definition as a fourth column.
    #[serde(default)]
    pub docs: Option<bool>,
    /// Print the source of each listed definition of up to 50 lines below its row (rows of
    /// `  N<tab>text`), unless another listed definition lies inside it. Default false.
    #[serde(default)]
    pub include_body: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrManyKinds {
    One(String),
    Many(Vec<String>),
}

impl From<OneOrManyKinds> for Vec<String> {
    fn from(v: OneOrManyKinds) -> Self {
        match v {
            OneOrManyKinds::One(s) => vec![s],
            OneOrManyKinds::Many(v) => v,
        }
    }
}

fn opt_kinds_map<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<BTreeMap<String, Vec<String>>>, D::Error> {
    Ok(Option::<BTreeMap<String, OneOrManyKinds>>::deserialize(d)?
        .map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect()))
}

/// The kind in `valid` that `raw` names, also by its common long names (`function`, `module`).
fn canonical_kind(valid: &[&'static str], raw: &str) -> Option<&'static str> {
    let lower = raw.to_ascii_lowercase();
    let wanted = match lower.as_str() {
        "function" | "func" | "method" => {
            if valid.contains(&"fn") {
                "fn"
            } else {
                "def"
            }
        }
        "module" => "mod",
        "constant" => "const",
        "interface" => "trait",
        "variable" => "var",
        other => other,
    };
    valid.iter().copied().find(|k| *k == wanted)
}

pub fn find_symbols(
    project: Option<&Project>,
    cfg: &ServerConfig,
    p: FindSymbolsParams,
) -> Result<String> {
    let queries = p.query.as_ref().map(Names::cleaned).unwrap_or_default();
    let selectors: Vec<SymbolSelector<'_>> = queries
        .iter()
        .map(|query| SymbolSelector::parse(query))
        .collect();
    let scope = p
        .scope
        .as_deref()
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(str::to_lowercase);
    let kinds_filter: Option<BTreeMap<String, Vec<String>>> = match p.kinds {
        None => None,
        Some(raw) if raw.is_empty() => None,
        Some(raw) => {
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
                let valid: Vec<&'static str> = syntax
                    .kinds()
                    .iter()
                    .chain(syntax.extra_kinds())
                    .copied()
                    .collect();
                let mut cleaned: Vec<String> = Vec::new();
                for raw in list.iter().map(|k| k.trim()).filter(|k| !k.is_empty()) {
                    let kind = canonical_kind(&valid, raw).ok_or_else(|| {
                        AgentError::invalid(
                            "kinds",
                            format!("unknown {key} kind `{raw}`; valid: {}", valid.join(", ")),
                        )
                    })?;
                    cleaned.push(kind.to_string());
                }
                map.entry(key).or_default().extend(cleaned);
            }
            Some(map)
        }
    };
    let target = ScanTarget {
        path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };
    let docs = p.docs.unwrap_or(false);
    let include_body = p.include_body.unwrap_or(false);
    // Per query: whether a definition matched it, and the close names for when none did.
    let mut matched = vec![false; queries.len()];
    let mut suggestions = vec![BTreeMap::<String, usize>::new(); queries.len()];
    let found = scan(project, &target, |lang, syntax, text, _| {
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
            None => !syntax.extra_kinds().contains(&symbol.kind),
        };
        let scope_allowed = |symbol: &Symbol| {
            scope
                .as_ref()
                .is_none_or(|scope| symbol.scope.to_string().to_lowercase().contains(scope))
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
        if include_body {
            symbols::attach_bodies(&mut listed, text);
        }
        Some(listed)
    })?;
    let mut out = found.render(cfg, &p.page, "no symbols", |group| {
        symbols::render(group, docs)
    })?;
    for (at, query) in queries.iter().enumerate().filter(|(at, _)| !matched[*at]) {
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
            (true, true) => format!("no symbols for `{query}`\n"),
            (true, false) => {
                format!("no symbols for `{query}`; similar symbol names: {near}\n")
            }
        });
    }
    Ok(out)
}
