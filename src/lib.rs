//! MCP server for coding agents, with three tool scopes:
//!
//! - [`workflow`]: session/turn state stored as Markdown in `.AGENTS/`, and atomic batches of
//!   `Create`/`Move`/`Patch`/`Delete`/`Recover` that are replayable as git-like diffs.
//! - [`filesystem`]: read-only listing, search and metadata tools that respect `.gitignore`.
//! - [`langtool`]: Tree-sitter code intelligence and the project's configured language tooling.
//!
//! Layout rule: dependencies point down this list: [`common`] holds errors, configuration, paths,
//! text and paging; [`language`] the languages the server knows and what each provides to the code
//! intelligence tools; [`toolchain`] the project's configured commands; then the scopes
//! [`filesystem`], [`langtool`] and [`workflow`]; [`server`] is the MCP surface that binds them.
//! `filesystem` and `langtool` reach the registered project through [`common::Workspace`], which
//! [`workflow::Project`] implements, so no dependency points up. Every scope has exactly two
//! folders: `tools/`, one file per MCP tool
//! (`<scope>_<name>` is `tools/<name>.rs`) with its `DESCRIPTION`, `<Tool>Params` and free handler
//! `fn <name>([project], [cfg], p)`, and `engine/`, the scope's logic and types, which never
//! import `tools/` (`Project` methods live only in `workflow/engine/`). A `DESCRIPTION` starts
//! with an imperative verb and says what the tool does, the shape of its output and the rules that
//! are not obvious; parameters are documented in their schema. Each item lives at the lowest
//! folder that covers all its users: one tool's file, the scope's `engine/`, or the root
//! [`common`]. A `mod.rs` only declares and re-exports. Integration tests mirror the scopes in
//! `tests/{workflow,filesystem,langtool}.rs` (`tests/<scope>/<tool>.rs` per tool) and
//! `tests/server.rs`, and share `tests/common`. `tests/layout.rs` checks the layer order, the
//! depth budget (layers three levels below `src/`, scopes four) and the `mod.rs` module style.

pub mod common;
pub mod filesystem;
pub mod langtool;
pub mod language;
pub mod server;
pub mod toolchain;
pub mod workflow;

pub use common::{AgentError, AgentUseMode, LineNumber, Range, ServerConfig};
pub use server::AgentServer;
