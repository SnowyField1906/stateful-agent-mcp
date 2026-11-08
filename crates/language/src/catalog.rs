//! Languages the server recognizes by file extension.

use std::path::Path;

/// A language the server recognizes by file extension. Its `name` is the section name in
/// `.AGENTS/config.toml`. Adding a language starts here: declare it and list it in
/// [`LANGUAGES`]; its code intelligence, if any, is a [`Syntax`](super::Syntax) that names it
/// (see the documentation of [`crate`]).
pub struct Language {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
}

pub static RUST: Language = Language {
    name: "rust",
    extensions: &["rs"],
};

pub static PYTHON: Language = Language {
    name: "python",
    extensions: &["py", "pyi"],
};

/// Every language the server recognizes.
pub static LANGUAGES: &[&Language] = &[&RUST, &PYTHON];

/// The language of a file, by extension.
pub fn language_for_path(path: &str) -> Option<&'static Language> {
    let ext = Path::new(path).extension()?.to_str()?;
    LANGUAGES
        .iter()
        .copied()
        .find(|l| l.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}
