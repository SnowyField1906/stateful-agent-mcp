//! `langtool_find_references`: where a name is defined and used, for languages with a
//! [`Syntax`](crate::langtool::Syntax). Output, grouped by file, one `line:role:text` per reference:
//! ```text
//! src/a.rs
//! 1:def:fn helper() {}
//! 4:call:helper();
//! ```

use std::cell::Cell;
use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::Deserialize;

use crate::common::params::PageParams;
use crate::common::{AgentError, Result, ServerConfig};
use crate::langtool::engine::callgraph::is_test_path;
use crate::langtool::engine::name_match::ranked_names;
use crate::langtool::engine::references::{self, Reference};
use crate::langtool::engine::scan::{ScanTarget, scan};
use crate::langtool::engine::syntax::SymbolSelector;
use crate::workflow::Project;

pub const DESCRIPTION: &str = "Find name-based references in code with Tree-sitter, ignoring comments and strings. A qualified `symbol` (`Type::name`) keeps references inside `Type` and uses that name it. Filter by lexical scope substring (`scope`); `include_scope` adds scope paths to results. Output per file: `line:role:text`, with role `def`, `import`, `call`, `type`, `field` or `use`; `role(as h)` marks a use through the import alias `h`. Matching remains name-based; lexical scope narrows candidates but does not resolve same-named bindings. References in test code are hidden unless `include_tests`, and the count of hidden ones is reported.";

/// Where a name is defined and used in code with a registered Tree-sitter grammar.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FindReferencesParams {
    /// Name to look up: a function, type, variable, field or module name. `Type::name`,
    /// `Type.name` or `Type/name` keeps the references inside `Type` and those that name it.
    pub symbol: String,
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

pub fn find_references(
    project: Option<&Project>,
    cfg: &ServerConfig,
    p: FindReferencesParams,
) -> Result<String> {
    let symbol = p.symbol.trim();
    let selector = SymbolSelector::parse(symbol);
    if selector.name.is_empty() {
        return Err(AgentError::invalid("symbol", "must not be empty"));
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
    let target = ScanTarget {
        path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };
    let found = scan(
        project,
        &target,
        |_, syntax, text, label| -> Option<Vec<Reference>> {
            let mut refs = references::references(syntax, text, selector.name)?;
            references::retain_qualified(&mut refs, &selector);
            if let Some(scope) = &scope {
                refs.retain(|reference| reference.scope.to_string().to_lowercase().contains(scope));
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
            Some(refs)
        },
    )?;
    let mut out = found.render(
        cfg,
        &p.page,
        &format!("no references to `{symbol}`"),
        |group| {
            if include_scope {
                references::render_with_scope(group)
            } else {
                references::render(group)
            }
        },
    )?;
    if hidden.get() > 0 {
        out.push_str(&format!(
            "({} references in tests hidden; set include_tests)\n",
            hidden.get()
        ));
    } else if out.starts_with("no references") {
        let names = scan(project, &target, |_, syntax, text, label| {
            if !include_tests && is_test_path(label) {
                Some(Vec::new())
            } else {
                references::identifier_names(syntax, text, include_tests)
            }
        })?;
        let (files, _) = names.into_parts();
        let candidates: BTreeSet<String> = files.into_iter().flat_map(|(_, names)| names).collect();
        let near = ranked_names(
            candidates.into_iter().collect::<Vec<_>>(),
            selector.name,
            5,
            false,
        );
        if !near.is_empty() {
            let near = near
                .into_iter()
                .map(|(_, name)| name)
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("hint: similar names: {near}\n"));
        }
    }
    Ok(out)
}
