//! Call trees for `langtool_view_callgraph`: indexes the functions below the target once, then
//! renders the tree of every requested name, or the closest names when a name has no definition.

use super::callgraph::{self, Direction, MAX_DEPTH, Query};
use super::index;
use super::scan::{ScanTarget, scan};
use crate::common::fs::resolve::{Scope, resolve};
use crate::common::text::bounded;
use crate::common::{AgentError, Result, Workspace, root_of};

/// The functions to trace and how far to follow their calls.
pub struct Request<'a> {
    /// Names, or qualified selectors such as `Type::name`.
    pub symbols: &'a [&'a str],
    /// The code that is indexed; callers outside it are not found.
    pub target: ScanTarget<'a>,
    /// Only the definition in this file, relative to the project root or absolute.
    pub file: Option<&'a str>,
    /// Only the definition that contains this 1-based line.
    pub line: Option<u64>,
    /// Callers, callees or both (the default).
    pub direction: Option<Direction>,
    /// Levels to follow; 1 by default, at most [`MAX_DEPTH`].
    pub tree_depth: Option<u64>,
    pub show_external: bool,
    pub include_tests: bool,
}

/// One tree per definition of every requested name, separated by blank lines.
pub fn view(workspace: Option<&dyn Workspace>, req: &Request<'_>) -> Result<String> {
    if req.symbols.is_empty() {
        return Err(AgentError::invalid("symbol", "must not be empty"));
    }
    let (files, skipped) = scan(workspace, &req.target, |_, syntax, text, _| {
        index::functions(syntax, text)
    })?
    .into_parts();
    let entries = index::entries(files);

    let scope = Scope::new(root_of(workspace), req.target.path, "implicit_path")?;
    let file_target = req
        .file
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(|f| resolve(&scope.root, Some(f)))
        .transpose()?;
    let file = file_target.as_ref().map(|t| t.rel.as_str());
    let mut out = String::new();
    for (at, &symbol) in req.symbols.iter().enumerate() {
        if at > 0 {
            out.push('\n');
        }
        let query = Query {
            symbol,
            file,
            line: req.line.map(|l| l as usize),
            direction: req.direction.unwrap_or(Direction::Both),
            depth: bounded(req.tree_depth, 1, 1, MAX_DEPTH as u64),
            show_external: req.show_external,
            include_tests: req.include_tests,
            root: &scope.root,
        };
        match callgraph::render(&entries, &query) {
            Some(trees) => out.push_str(&trees),
            None => {
                out.push_str(&format!("no function `{symbol}` found"));
                let near = callgraph::suggest(&entries, symbol);
                if !near.is_empty() {
                    out.push_str(&format!("; similar names: {}", near.join(", ")));
                }
                if skipped > 0 {
                    out.push_str(&format!(
                        " ({skipped} unreadable, binary or oversized files skipped)"
                    ));
                }
                out.push('\n');
            }
        }
    }
    Ok(out)
}
