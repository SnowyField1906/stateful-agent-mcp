//! `langtool_find_references`: where a name is defined and used, for languages with a
//! [`Syntax`](crate::language::Syntax). Output, grouped by file, one `line:role:text` per reference:
//! ```text
//! src/a.rs
//! 1:def:fn helper() {}
//! 4:call:helper();
//! ```

use schemars::JsonSchema;
use serde::Deserialize;

use crate::common::params::{Names, PageParams};
use crate::common::{AgentError, Result, ServerConfig, Workspace};
use crate::langtool::engine::incoming::{self, Incoming};
use crate::langtool::engine::outgoing::{self, Outgoing};
use crate::langtool::engine::scan::ScanTarget;
use crate::langtool::engine::scope::scope_filter;

pub const DESCRIPTION: &str = "Find name-based references in code with Tree-sitter, ignoring comments and strings. `symbol` is one name or a list, all searched in one scan; with a list, each file's rows are grouped under `# name` lines. A qualified `symbol` (`Type::name`) keeps references inside `Type` and those whose line names it. Filter by lexical scope substring (`scope`); `include_scope` adds scope paths to results. Output per file: `line:role:text`, with role `def`, `import`, `call`, `type`, `field` or `use`; `role(as h)` marks a use through the import alias `h`. Matching remains name-based; lexical scope narrows candidates but does not resolve same-named bindings. References in test code are hidden unless `include_tests`, and the count of hidden ones is reported. Give `from_symbol` (a name or list) instead of `symbol` for the opposite direction: the definition's row, then one row `role<tab>name<tab>@lines<tab>definitions` per name its signature and body use, where definitions are the project's (`file:line kind Qualified::name`), and an `external` row for the called or type names that no indexed file defines, summarized by where they come from as in `langtool_view_callgraph` (`3 std, 1 unresolved; deps: serde_json; project: helper`). Names the definition defines itself, imports, fields and uses that match no project definition (usually locals) are left out.";

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

pub fn find_references(
    workspace: Option<&dyn Workspace>,
    cfg: &ServerConfig,
    p: FindReferencesParams,
) -> Result<String> {
    let symbols = p.symbol.cleaned();
    let from = p
        .from_symbol
        .as_ref()
        .map(Names::cleaned)
        .unwrap_or_default();
    let target = ScanTarget {
        path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };
    let scope = scope_filter(p.scope.as_deref());
    let include_tests = p.include_tests.unwrap_or(false);
    if from.is_empty() {
        let query = Incoming {
            symbols: &symbols,
            target,
            scope,
            include_tests,
            include_scope: p.include_scope.unwrap_or(false),
            page: &p.page,
        };
        return incoming::find(workspace, cfg, &query);
    }
    if !symbols.is_empty() {
        return Err(AgentError::invalid(
            "from_symbol",
            "give `symbol` or `from_symbol`, not both",
        ));
    }
    let query = Outgoing {
        from: &from,
        target,
        scope,
        include_tests,
        page: &p.page,
    };
    outgoing::find(workspace, cfg, &query)
}
