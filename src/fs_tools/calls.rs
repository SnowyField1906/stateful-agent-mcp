//! `code_calls`: the call graph of a function or method, for languages with a
//! [`Syntax`](crate::lang::Syntax). Calls are matched to definitions by name across the indexed
//! files. Output, one tree per matching definition:
//! ```text
//! a	src/a.rs:1..4
//! outgoing:
//!   b	src/a.rs:6..8
//!   ext	(external)
//! incoming:
//!   c	src/c.rs:1..3
//! ```

use crate::config::ServerConfig;
use crate::error::{AgentError, Result};
use crate::lang::calls::{self, Direction, Entry, MAX_DEPTH, Query};
use crate::params::CodeCallsParams;
use crate::state::Project;

use super::fmt::bounded;
use super::scan::{ScanTarget, scan};

pub fn code_calls(project: Option<&Project>, cfg: &ServerConfig, p: CodeCallsParams) -> Result<String> {
    let symbol = p.symbol.trim();
    if symbol.is_empty() {
        return Err(AgentError::invalid("symbol", "must not be empty"));
    }
    let direction = match p.direction.as_deref().map(|d| d.trim().to_ascii_lowercase()).as_deref() {
        None | Some("" | "both") => Direction::Both,
        Some("incoming") => Direction::Incoming,
        Some("outgoing") => Direction::Outgoing,
        Some(other) => {
            return Err(AgentError::invalid(
                "direction",
                format!("`{other}`; expected incoming, outgoing or both"),
            ));
        }
    };

    let target = ScanTarget {
        implicit_path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };
    let (files, skipped) = scan(project, &target, |_, syntax, text| calls::functions(syntax, text))?.into_parts();
    let index: Vec<Entry> = files
        .into_iter()
        .flat_map(|(file, funcs)| {
            funcs.into_iter().map(move |func| Entry { file: file.clone(), func })
        })
        .collect();

    let query = Query {
        symbol,
        file: p.file_path.as_deref().map(str::trim).filter(|f| !f.is_empty()),
        line: p.line.map(|l| l as usize),
        direction,
        depth: bounded(p.tree_depth, 1, 1, MAX_DEPTH as u64),
    };
    Ok(calls::render(&index, &query).unwrap_or_else(|| {
        let mut out = format!("no function `{symbol}` found");
        if skipped > 0 {
            out.push_str(&format!(" ({skipped} unreadable, binary or oversized files skipped)"));
        }
        out.push('\n');
        out
    }))
}
