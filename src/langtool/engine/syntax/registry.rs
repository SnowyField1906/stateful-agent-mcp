//! Which languages have code intelligence: the [`Syntax`] of each language in
//! [`crate::toolchain::LANGUAGES`]. Adding a language's syntax starts here.

use crate::toolchain::{Language, language_for_path};

use super::Syntax;
use super::python::PythonSyntax;
use super::rust::RustSyntax;

/// The syntax of `language`; `None` when only its configured tooling is used.
pub fn of(language: &Language) -> Option<&'static dyn Syntax> {
    match language.name {
        "rust" => Some(&RustSyntax),
        "python" => Some(&PythonSyntax),
        _ => None,
    }
}

/// Languages with a Tree-sitter adapter, in the same order as the toolchain catalog.
pub fn supported_languages() -> impl Iterator<Item = &'static Language> {
    crate::toolchain::LANGUAGES
        .iter()
        .filter(|language| of(language).is_some())
}

/// Names shown in validation errors for language-specific queries.
pub fn supported_names() -> String {
    supported_languages()
        .map(|language| language.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Names and file extensions shown when a path has no Tree-sitter adapter.
pub fn supported_file_types() -> String {
    supported_languages()
        .map(|language| {
            let extensions = language
                .extensions
                .iter()
                .map(|extension| format!(".{extension}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{} ({extensions})", language.name)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The syntax of the file at `path`, by extension.
pub fn for_path(path: &str) -> Option<&'static dyn Syntax> {
    language_for_path(path).and_then(of)
}
