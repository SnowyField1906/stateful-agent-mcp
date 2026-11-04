//! Language-aware tooling: project configuration (`.AGENTS/config.toml`) and, in later
//! modules, formatting, project actions and Tree-sitter code intelligence.

pub mod actions;
pub mod aliases;
pub mod calls;
pub mod config;
pub mod depth;
pub mod exec;
pub mod format;
pub mod references;
pub mod symbols;
pub mod syntax;

use std::path::Path;

pub use config::{Action, LanguageConfig, LanguageTooling};
pub use syntax::Syntax;

/// A language the server recognizes by file extension. Its `name` is the section name in
/// `.AGENTS/config.toml`. Adding a language starts here.
pub struct Language {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    /// Code intelligence for the language; `None` when only its configured tooling is used.
    pub syntax: Option<&'static dyn Syntax>,
}

pub const LANGUAGES: &[Language] = &[
    Language { name: "rust", extensions: &["rs"], syntax: Some(&syntax::rust::RustSyntax) },
    Language { name: "python", extensions: &["py", "pyi"], syntax: Some(&syntax::python::PythonSyntax) },
];

/// The language of a file, by extension.
pub fn language_for_path(path: &str) -> Option<&'static Language> {
    let ext = Path::new(path).extension()?.to_str()?;
    LANGUAGES
        .iter()
        .find(|l| l.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_language_by_extension() {
        assert_eq!(language_for_path("src/a.rs").map(|l| l.name), Some("rust"));
        assert_eq!(language_for_path("x/y.PYI").map(|l| l.name), Some("python"));
        assert!(language_for_path("README.md").is_none());
        assert!(language_for_path("Makefile").is_none());
    }
}
