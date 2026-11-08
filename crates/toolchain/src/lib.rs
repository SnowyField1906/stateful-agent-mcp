//! The project's toolchain, configured in `.AGENTS/config.toml`: lint, build, test, install and
//! remove actions, command execution and formatting. The languages themselves are described in
//! [`crate::language`].

use stateful_agent_common as common;
use stateful_agent_language as language;

pub mod actions;
pub mod config;
pub mod exec;
pub mod format;

pub use config::{Action, LanguageConfig, LanguageTooling};
