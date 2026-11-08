//! The `stateful-agent` MCP server over stdio: the entry point that serves the layer crates under
//! `crates/`, each with its own tests in `tests/`.
//!
//! The tool scopes are `workflow` (session/turn state stored as Markdown in `.AGENTS/`, and atomic
//! batches of `Create`/`Move`/`Patch`/`Delete`/`Recover` that are replayable as git-like diffs),
//! `filesystem` (read-only listing, search and metadata tools that respect `.gitignore`) and
//! `langtool` (Tree-sitter code intelligence and the project's configured language tooling). The
//! `server` crate binds them to one shared state. This crate exports only what the binary needs.

use stateful_agent_common as common;
use stateful_agent_server as server;

pub use common::{AgentUseMode, ServerConfig};
pub use server::AgentServer;
