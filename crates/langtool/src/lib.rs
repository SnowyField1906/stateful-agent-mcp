//! Langtool scope: Tree-sitter code intelligence for registered languages, and running the
//! project's configured language commands.
//!
//! - `tools/`: one file per `langtool_*` tool, holding its parameters and handler.
//! - `engine/`: the scan and the Tree-sitter analysis behind the code intelligence tools; a
//!   language plugs in through [`crate::language::Syntax`]. The configured commands live in
//!   [`crate::toolchain`].

use stateful_agent_common as common;
use stateful_agent_language as language;
use stateful_agent_toolchain as toolchain;

pub mod engine;
pub mod tools;

pub use engine::fold::FoldLevel;
