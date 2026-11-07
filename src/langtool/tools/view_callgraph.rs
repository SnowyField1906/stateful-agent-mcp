//! `langtool_view_callgraph`: the call graph of a function or method, for languages with a
//! [`Syntax`](crate::langtool::Syntax). Calls are matched to definitions by name across the indexed
//! files. Output, one tree per matching definition:
//! ```text
//! a	src/a.rs:1..4
//! outgoing:
//!   b	src/a.rs:6..8
//!   ext	(external)
//! incoming:
//!   c	src/c.rs:1..3
//! ```

use schemars::JsonSchema;
use serde::Deserialize;

use crate::common::fs::resolve::{Scope, resolve};
use crate::common::params::{Names, flex_u64};
use crate::common::text::bounded;
use crate::common::{AgentError, Result, ServerConfig};
use crate::langtool::engine::callgraph::{self, Direction, Entry, MAX_DEPTH, Query};
use crate::langtool::engine::scan::{ScanTarget, scan};
use crate::workflow::Project;

pub const DESCRIPTION: &str = "Show the call graph of a function or method by name or a list of names (`Type::method` selects that type's method), parsed with a registered Tree-sitter grammar: what it calls (`outgoing`) and what calls it (`incoming`), as one tree per definition of lines `name (in scope)<tab>file:A..B`; call edges end with `@N`, the line of the call in the calling function. Only code under `implicit_path` is indexed, once for all names, so callers outside it are not found. A call is matched by how it is written: `self.f()` and `Self::f()` stay in the enclosing type, `Type::f()` goes to that type, `module::f()` to free functions of that module, `f()` to free functions, Rust resolves method receivers from typed parameters or simple local bindings when possible; receivers typed `&dyn Trait`, `Box<dyn Trait>` or `impl Trait` reach the trait's declaration and every `impl Trait for T`, and `Box`, `Rc` and `Arc` give the methods of what they hold; unresolved receivers remain `?` candidates. Constructors such as `Some(x)` are not calls. Calls without a definition in the indexed code are summarized as `(external: ...)`, listed in full with `show_external`. `(cycle)` marks a call back to a definition already on the path. Tests are left out unless `include_tests`. `file_path` or `line` picks one of several definitions with the same name.";

/// What a function or method calls and what calls it in code with a registered Tree-sitter grammar.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ViewCallgraphParams {
    /// Name of the function or method, or a list of names indexed in one scan. `Type::name`,
    /// `Type.name` or `Type/name` selects the definition inside `Type`.
    #[schemars(with = "Vec<String>")]
    pub symbol: Names,
    /// File or directory to index, relative to the project root or absolute. Default `.`.
    #[serde(default)]
    pub implicit_path: Option<String>,
    /// Only the definition in this file, as the output shows it, when several share the name.
    #[serde(default)]
    pub file_path: Option<String>,
    /// Only the definition that contains this 1-based line.
    #[serde(default, deserialize_with = "flex_u64")]
    pub line: Option<u64>,
    /// `incoming` (callers), `outgoing` (callees) or `both` (default).
    #[serde(default)]
    pub direction: Option<Direction>,
    /// Call tree levels to follow (default 1, max 3).
    #[serde(default, deserialize_with = "flex_u64")]
    pub tree_depth: Option<u64>,
    /// List every call that has no definition in the indexed code. Default false: one summary
    /// line per function.
    #[serde(default)]
    pub show_external: Option<bool>,
    /// Include callers and callees that are tests (default false).
    #[serde(default)]
    pub include_tests: Option<bool>,
    /// Include dotfiles and gitignored files.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
}

pub fn view_callgraph(
    project: Option<&Project>,
    cfg: &ServerConfig,
    p: ViewCallgraphParams,
) -> Result<String> {
    let symbols = p.symbol.cleaned();
    if symbols.is_empty() {
        return Err(AgentError::invalid("symbol", "must not be empty"));
    }
    let direction = p.direction.unwrap_or(Direction::Both);

    let target = ScanTarget {
        path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };
    let (files, skipped) = scan(project, &target, |_, syntax, text, _| {
        callgraph::functions(syntax, text)
    })?
    .into_parts();
    let index: Vec<Entry> = files
        .into_iter()
        .flat_map(|(file, funcs)| {
            funcs.into_iter().map(move |func| Entry {
                file: file.clone(),
                func,
            })
        })
        .collect();

    let scope = Scope::new(project.map(Project::root_dir), target.path, "implicit_path")?;
    let file_target = p
        .file_path
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(|f| resolve(&scope.root, Some(f)))
        .transpose()?;
    let file = file_target.as_ref().map(|t| t.rel.as_str());
    let mut out = String::new();
    for (at, &symbol) in symbols.iter().enumerate() {
        if at > 0 {
            out.push('\n');
        }
        let query = Query {
            symbol,
            file,
            line: p.line.map(|l| l as usize),
            direction,
            depth: bounded(p.tree_depth, 1, 1, MAX_DEPTH as u64),
            show_external: p.show_external.unwrap_or(false),
            include_tests: p.include_tests.unwrap_or(false),
        };
        match callgraph::render(&index, &query) {
            Some(trees) => out.push_str(&trees),
            None => {
                out.push_str(&format!("no function `{symbol}` found"));
                let near = callgraph::suggest(&index, symbol);
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
