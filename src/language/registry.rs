//! Which languages have code intelligence: the [`Syntax`] of each. Registering a language is one
//! line in [`SYNTAXES`].

use super::python::PythonSyntax;
use super::rust::RustSyntax;
use super::{Language, Syntax, language_for_path};

/// The syntax of every supported language, in the order the languages are named in messages.
static SYNTAXES: &[&dyn Syntax] = &[&RustSyntax, &PythonSyntax];

/// The syntax of `language`; `None` when only its configured tooling is used.
pub fn of(language: &Language) -> Option<&'static dyn Syntax> {
    SYNTAXES
        .iter()
        .copied()
        .find(|syntax| syntax.language().name == language.name)
}

/// Languages with a Tree-sitter adapter.
pub fn supported_languages() -> impl Iterator<Item = &'static Language> {
    SYNTAXES.iter().map(|syntax| syntax.language())
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
