//! Shared foundation: errors, configuration, ranges and paging, path specs, text helpers, atomic
//! writes, lenient parameter parsing, and path resolution and directory walking. Knows no scope
//! and no project state.
//!
//! Layers, each depending only on the ones before it:
//! `common` → `language` → `toolchain` → `filesystem` → `langtool` → `workflow` → `server`.
//! [`Workspace`] is how the scopes below `workflow` reach the registered project without
//! importing it.

pub mod align;
pub mod config;
pub mod error;
pub mod fs;
pub mod io;
pub mod params;
pub mod pathspec;
pub mod range;
pub mod text;
pub mod workspace;

pub use config::{AgentUseMode, ServerConfig};
pub use error::{AgentError, Result};
pub use params::{Names, PageParams};
pub use range::{LineNumber, Range};
pub use workspace::{Front, ItemKind, Moved, OpenItem, Workspace, root_of};
