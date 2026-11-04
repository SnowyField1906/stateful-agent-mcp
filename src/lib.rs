//! MCP server for coding agents.
//!
//! - Session/turn state is stored as Markdown in `.AGENTS/` (see [`session`]).
//! - File changes go through atomic batches of `Create`/`Move`/`Patch`/`Delete`/`Recover`
//!   (see [`files`]) and are replayable as git-like diffs (see [`state`]).
//! - Read-only filesystem tools respect `.gitignore` (see [`fs_tools`]).

pub mod config;
pub mod error;
pub mod files;
pub mod fs_tools;
pub mod instructions;
pub mod lang;
pub mod output;
pub mod params;
pub mod server;
pub mod session;
pub mod state;

pub use config::{AgentUseMode, ServerConfig};
pub use error::AgentError;
pub use server::CodingAgentServer;
