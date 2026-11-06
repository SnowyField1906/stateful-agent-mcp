//! The project's toolchain, configured in `.AGENTS/config.toml`: the languages the server
//! recognizes, lint, build, test, install and remove actions, command execution and formatting.

pub mod actions;
pub mod config;
pub mod exec;
pub mod format;
pub mod language;

pub use config::{Action, LanguageConfig, LanguageTooling};
pub use language::{LANGUAGES, Language, language_for_path};
