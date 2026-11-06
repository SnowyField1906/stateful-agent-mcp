//! Languages the server recognizes by file extension.

use std::path::Path;

/// A language the server recognizes by file extension. Its `name` is the section name in
/// `.AGENTS/config.toml`. Adding a language starts here; its code intelligence, if any, is
/// attached in `langtool::engine::syntax::registry`.
pub struct Language {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
}

pub const LANGUAGES: &[Language] = &[
    Language {
        name: "rust",
        extensions: &["rs"],
    },
    Language {
        name: "python",
        extensions: &["py", "pyi"],
    },
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
