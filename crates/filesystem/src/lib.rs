//! Filesystem scope: read-only listing, search and metadata tools that respect `.gitignore`.
//!
//! - `tools/`: one file per `filesystem_*` tool, holding its parameters and handler.
//! - `engine/`: the logic and types that more than one tool uses: the listing behind `ls` and
//!   `find`.
//!
//! Path resolution and directory walking, which `langtool` uses too, live in [`crate::common::fs`].

use stateful_agent_common as common;

mod engine;
pub mod tools;
