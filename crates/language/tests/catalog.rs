//! The catalog and the registry: every language is named once and found by its file extensions.

use std::collections::HashSet;

use stateful_agent_language::{LANGUAGES, language_for_path, registry};

use super::support::syntaxes;

#[test]
fn names_and_extensions_identify_one_language_each() {
    let mut names = HashSet::new();
    let mut extensions = HashSet::new();
    for language in LANGUAGES {
        let name = language.name;
        assert!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "`{name}` is not a lowercase name"
        );
        assert!(names.insert(name), "`{name}` is listed twice");
        assert!(!language.extensions.is_empty(), "{name}: no extensions");
        for extension in language.extensions {
            assert!(
                !extension.is_empty()
                    && !extension.contains('.')
                    && *extension == extension.to_ascii_lowercase(),
                "{name}: `{extension}` is not a lowercase extension without a dot"
            );
            assert!(
                extensions.insert(*extension),
                "`.{extension}` belongs to two languages"
            );
        }
    }
}

#[test]
fn every_extension_finds_its_language_whatever_its_case() {
    for language in LANGUAGES {
        for extension in language.extensions {
            for file in [
                format!("src/sample.{extension}"),
                format!("SAMPLE.{}", extension.to_ascii_uppercase()),
            ] {
                assert_eq!(
                    language_for_path(&file).map(|found| found.name),
                    Some(language.name),
                    "{file}"
                );
            }
        }
    }
    assert!(language_for_path("README").is_none());
}

#[test]
fn a_registered_syntax_is_a_catalog_language_registered_once() {
    let mut seen = HashSet::new();
    for syntax in syntaxes() {
        let language = syntax.language();
        assert!(
            LANGUAGES.iter().any(|known| known.name == language.name),
            "{}: the syntax names a language that LANGUAGES does not list",
            language.name
        );
        assert!(
            seen.insert(language.name),
            "{}: two syntaxes",
            language.name
        );
        assert_eq!(
            registry::of(language).map(|found| found.language().name),
            Some(language.name)
        );
        for extension in language.extensions {
            let found = registry::for_path(&format!("sample.{extension}"));
            assert_eq!(
                found.map(|found| found.language().name),
                Some(language.name),
                "{}: `.{extension}` does not reach the syntax",
                language.name
            );
        }
    }
}

#[test]
fn messages_list_every_supported_language() {
    let names = registry::supported_names();
    let types = registry::supported_file_types();
    for syntax in syntaxes() {
        let language = syntax.language();
        assert!(names.contains(language.name), "{names}");
        for extension in language.extensions {
            assert!(types.contains(&format!(".{extension}")), "{types}");
        }
    }
}
