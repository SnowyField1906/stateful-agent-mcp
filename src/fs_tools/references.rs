//! `code_references`: where a name is defined and used, for languages with a
//! [`Syntax`](crate::lang::Syntax). Output, grouped by file:
//! ```text
//! src/a.rs
//! 1:def:fn helper() {}
//! 4:helper();
//! ```

use crate::config::ServerConfig;
use crate::error::{AgentError, Result};
use crate::lang::references::{self, Reference};
use crate::params::CodeReferencesParams;
use crate::state::Project;

use super::scan::{ScanTarget, scan};

pub fn code_references(project: Option<&Project>, cfg: &ServerConfig, p: CodeReferencesParams) -> Result<String> {
    let symbol = p.symbol.trim();
    if symbol.is_empty() {
        return Err(AgentError::invalid("symbol", "must not be empty"));
    }
    let target = ScanTarget {
        implicit_path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };
    let found = scan(project, &target, |_, syntax, text| -> Option<Vec<Reference>> {
        references::references(syntax, text, symbol)
    })?;
    found.render(
        cfg,
        &p.page,
        &format!("no references to `{symbol}`"),
        |group| references::render(group),
    )
}
