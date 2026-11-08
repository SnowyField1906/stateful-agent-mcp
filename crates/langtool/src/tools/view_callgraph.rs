//! `langtool_view_callgraph`: the call graph of a function or method, for languages with a
//! [`Syntax`](crate::language::Syntax). Calls are matched to definitions by name across the indexed
//! files. Output, one tree per matching definition:
//! ```text
//! a	src/a.rs:1..4
//! outgoing:
//!   b	src/a.rs:6..8
//!   (external: 1 std)
//! incoming:
//!   c	src/c.rs:1..3
//! ```

use schemars::JsonSchema;
use serde::Deserialize;

use crate::common::params::{Names, flex_u64};
use crate::common::{Result, ServerConfig, Workspace};
use crate::engine::callgraph::Direction;
use crate::engine::calltree::{self, Request};
use crate::engine::scan::ScanTarget;

pub const DESCRIPTION: &str = "Show the call graph of a function or method by name or a list of names (`Type::method` selects that type's method), parsed with a registered Tree-sitter grammar: what it calls (`outgoing`) and what calls it (`incoming`), as one tree per definition of lines `name (in scope)<tab>file:A..B`; call edges end with `@N`, the line of the call in the calling function. Only code under `implicit_path` is indexed, once for all names, so callers outside it are not found. A call is matched by how it is written: `self.f()` and `Self::f()` stay in the enclosing type, `Type::f()` goes to that type, `module::f()` to free functions of that module, `f()` to free functions, Rust resolves method receivers from typed parameters or simple local bindings when possible; receivers typed `&dyn Trait`, `Box<dyn Trait>` or `impl Trait` reach the trait's declaration and every `impl Trait for T`, and `Box`, `Rc` and `Arc` give the methods of what they hold; unresolved receivers remain `?` candidates, except a method whose name is also called on a standard library or dependency receiver elsewhere in the scan, which is counted as unresolved (the queried name is exempt). Constructors such as `Some(x)` are not calls. Calls without a definition in the indexed code go on one `(external: ...)` line per function, placed from the imports of their file and not from a list of names: `N std` (standard library, prelude, builtins), `N unresolved` (such as methods on a value of unknown type), `deps: a, b` (packages outside the project, which is the nearest manifest root such as the folder of a `Cargo.toml` or its workspace root, or in ignored files; a sibling project is one) and `project: x` (project code the scan did not reach); `show_external` lists each call with that tag, and a call that an import places in the standard library or a dependency never links to a same-named project function. `(cycle)` marks a call back to a definition already on the path. Tests are left out unless `include_tests`. `file_path` or `line` picks one of several definitions with the same name.";

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
    /// List every call that has no definition in the indexed code, each tagged `(std)`,
    /// `(dep name)`, `(project)` or `(unresolved)`. Default false: one summary line per function.
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
    workspace: Option<&dyn Workspace>,
    cfg: &ServerConfig,
    p: ViewCallgraphParams,
) -> Result<String> {
    let symbols = p.symbol.cleaned();
    let request = Request {
        symbols: &symbols,
        target: ScanTarget {
            path: p.implicit_path.as_deref(),
            max_depth: cfg.max_dir_depth,
            show_gitignored: p.show_gitignored,
        },
        file: p.file_path.as_deref(),
        line: p.line,
        direction: p.direction,
        tree_depth: p.tree_depth,
        show_external: p.show_external.unwrap_or(false),
        include_tests: p.include_tests.unwrap_or(false),
    };
    calltree::view(workspace, &request)
}
