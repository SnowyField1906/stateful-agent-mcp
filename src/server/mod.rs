//! MCP surface: the shared [`State`], the bindings of each scope's tools to it, the tool handler
//! and result conversion. A tool's description is the `DESCRIPTION` of its file in the scope's
//! `tools/`.

mod bindings;
mod handler;
mod hints;
mod output;
mod state;
mod steering;

pub use handler::AgentServer;
pub use state::State;
