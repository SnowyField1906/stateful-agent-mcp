//! MCP surface: the shared [`State`], the bindings of each scope's tools to it, the tool handler
//! and result conversion. A tool's description is the `DESCRIPTION` of its file in the scope's
//! `tools/`.

use stateful_agent_common as common;
use stateful_agent_filesystem as filesystem;
use stateful_agent_langtool as langtool;
use stateful_agent_language as language;
use stateful_agent_toolchain as toolchain;
use stateful_agent_workflow as workflow;

mod bindings;
pub mod handler;
pub mod hints;
pub mod output;
pub mod state;
pub mod steering;

pub use handler::AgentServer;
pub use state::State;
