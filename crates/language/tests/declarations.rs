//! `Declarations`: the table of kinds is consistent, and the definitions of the sample use it.

use std::collections::HashSet;
use std::ptr;

use stateful_agent_language::{Definition, Syntax, node_text};
use tree_sitter::Node;

use super::support::{Sample, is_inside, name, syntaxes};

/// The definitions of the sample with the nodes that introduce them.
fn definitions<'a>(syntax: &dyn Syntax, sample: &'a Sample) -> Vec<(Node<'a>, Definition)> {
    sample
        .nodes()
        .into_iter()
        .filter_map(|node| {
            syntax
                .definition(node, &sample.source)
                .map(|definition| (node, definition))
        })
        .collect()
}

#[test]
fn the_kinds_form_a_consistent_table() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let kinds = syntax.kinds();
        assert!(!kinds.is_empty(), "{language}: no kinds");

        let mut words = HashSet::new();
        for kind in kinds {
            assert!(
                !kind.name.is_empty() && kind.name.chars().all(|c| c.is_ascii_lowercase()),
                "{language}: `{}` is not a lowercase word",
                kind.name
            );
            for word in std::iter::once(&kind.name).chain(kind.aliases) {
                assert!(
                    words.insert(word.to_ascii_lowercase()),
                    "{language}: `{word}` names two kinds"
                );
            }
            assert!(kind.is_named(kind.name));
            assert!(
                !kind.interface || kind.owner,
                "{language}: the interface `{}` holds no methods",
                kind.name
            );
        }

        // The kinds that are always listed come first, then the ones listed on request.
        if let Some(first) = kinds.iter().position(|kind| kind.on_request) {
            assert!(
                kinds[first..].iter().all(|kind| kind.on_request),
                "{language}: a kind listed always follows one listed on request"
            );
        }
        assert!(
            kinds.iter().any(|kind| kind.function),
            "{language}: no function kind"
        );
        assert!(
            kinds.iter().any(|kind| kind.declares_type),
            "{language}: no kind declares a type"
        );
        assert!(
            kinds.iter().any(|kind| kind.owner),
            "{language}: no kind holds methods"
        );
    }
}

#[test]
fn definitions_of_the_sample_are_of_a_listed_kind_and_the_sample_defines_each_role() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let sample = Sample::of(syntax);
        let found = definitions(syntax, &sample);

        for (_, definition) in &found {
            assert!(
                syntax
                    .kinds()
                    .iter()
                    .any(|kind| ptr::eq(*kind, definition.kind)),
                "{language}: `{}` is of a kind that `kinds` does not list",
                definition.name
            );
            assert!(
                !definition.name.is_empty() && !definition.name.contains(char::is_whitespace),
                "{language}: a {} is named {:?}",
                definition.kind,
                definition.name
            );
            assert!(
                definition.trait_name.is_none() || definition.kind.owner,
                "{language}: `{}` implements a trait but is not an owner",
                definition.name
            );
        }

        assert!(
            found.iter().any(|(_, definition)| definition.kind.function),
            "{language}: the sample defines no function"
        );
        assert!(
            found
                .iter()
                .any(|(_, definition)| definition.kind.declares_type),
            "{language}: the sample defines no type"
        );
        let owner_with_method = found.iter().any(|(owner, definition)| {
            definition.kind.owner
                && found.iter().any(|(inner, other)| {
                    other.kind.function && inner.id() != owner.id() && is_inside(*inner, *owner)
                })
        });
        assert!(
            owner_with_method,
            "{language}: the sample has no type that holds a method"
        );
    }
}

#[test]
fn functions_and_types_are_named_by_a_name_field() {
    for syntax in syntaxes() {
        let language = name(syntax);
        assert!(
            !syntax.name_fields().is_empty() && syntax.name_fields().iter().all(|f| !f.is_empty()),
            "{language}: no name fields"
        );
        let sample = Sample::of(syntax);
        for (node, definition) in definitions(syntax, &sample) {
            if !(definition.kind.function || definition.kind.declares_type) {
                continue;
            }
            let named = syntax
                .name_fields()
                .iter()
                .filter_map(|field| node.child_by_field_name(*field))
                .any(|child| node_text(child, &sample.source) == definition.name);
            assert!(
                named,
                "{language}: no name field of `{}` holds its name",
                definition.name
            );
        }
    }
}

#[test]
fn a_documentation_line_is_one_line_without_comment_markers() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let sample = Sample::of(syntax);
        let lines: Vec<String> = definitions(syntax, &sample)
            .iter()
            .filter_map(|(node, _)| syntax.doc_line(*node, &sample.source))
            .collect();
        assert!(
            !lines.is_empty(),
            "{language}: no definition of the sample has a documentation line"
        );
        for line in &lines {
            assert!(
                !line.is_empty()
                    && line.trim() == line
                    && !line.contains('\n')
                    && !line.starts_with(['/', '#', '"', '\'']),
                "{language}: the documentation line {line:?} keeps whitespace or markers"
            );
        }
    }
}

#[test]
fn the_unit_of_a_definition_holds_the_definition() {
    for syntax in syntaxes() {
        let sample = Sample::of(syntax);
        for (node, definition) in definitions(syntax, &sample) {
            assert!(
                is_inside(node, syntax.unit_node(node)),
                "{}: the unit of `{}` does not hold it",
                name(syntax),
                definition.name
            );
        }
    }
}

#[test]
fn names_and_paths_follow_the_conventions_of_the_language() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let extension = syntax.language().extensions[0];
        assert!(
            !syntax.is_type_name(""),
            "{language}: an empty name is a type"
        );
        assert!(
            syntax.is_test_path(&format!("tests/sample.{extension}")),
            "{language}: a file in `tests/` is not test code"
        );
        assert!(
            !syntax.is_test_path(&format!("src/sample.{extension}")),
            "{language}: a file in `src/` is test code"
        );
    }
}

#[test]
fn the_sample_has_exported_and_private_definitions() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let sample = Sample::of(syntax);
        let found = definitions(syntax, &sample);
        let exported = found
            .iter()
            .filter(|(node, _)| syntax.exported(*node, &sample.source))
            .count();
        assert!(exported > 0, "{language}: the sample exports nothing");
        assert!(
            exported < found.len(),
            "{language}: every definition of the sample is exported"
        );
    }
}
