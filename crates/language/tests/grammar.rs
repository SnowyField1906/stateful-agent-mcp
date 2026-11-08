//! `Grammar`: the grammar loads, and the node kinds the folded view and the name searches rely on
//! exist in its trees.

use std::collections::HashSet;

use stateful_agent_language::{Ident, parse};

use super::support::{Sample, name, syntaxes};

#[test]
fn the_grammar_parses_an_empty_file_and_the_sample() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let empty =
            parse(syntax, "").unwrap_or_else(|| panic!("{language}: the grammar does not load"));
        assert!(
            !empty.root_node().has_error(),
            "{language}: an empty file is a syntax error"
        );
        let sample = Sample::of(syntax);
        assert!(
            !sample.tree.root_node().has_error(),
            "{language}: the sample has a syntax error"
        );
    }
}

#[test]
fn the_declared_node_kinds_exist_in_the_grammar() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let grammar = syntax.grammar();
        for list in [syntax.block_kinds(), syntax.text_kinds()] {
            assert!(
                !list.is_empty(),
                "{language}: a list of node kinds is empty"
            );
            let distinct: HashSet<_> = list.iter().collect();
            assert_eq!(
                distinct.len(),
                list.len(),
                "{language}: {list:?} repeats a kind"
            );
            for kind in list {
                assert_ne!(
                    grammar.id_for_node_kind(kind, true),
                    0,
                    "{language}: `{kind}` is not a named node kind of the grammar"
                );
            }
        }
    }
}

#[test]
fn the_elision_is_one_line_without_line_numbers() {
    for syntax in syntaxes() {
        let text = syntax.elision();
        assert!(
            !text.is_empty() && !text.contains('\n') && !text.chars().any(|c| c.is_ascii_digit()),
            "{}: the elision {text:?} must be one line without digits",
            name(syntax)
        );
    }
}

#[test]
fn identifiers_are_named_leaves_that_are_not_text() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let sample = Sample::of(syntax);
        let mut values = 0;
        for node in sample.nodes() {
            let Some(ident) = syntax.identifier(node) else {
                continue;
            };
            assert!(
                node.is_named() && node.named_child_count() == 0,
                "{language}: the identifier `{}` is not a named leaf",
                node.kind()
            );
            assert!(
                !syntax.text_kinds().contains(&node.kind()),
                "{language}: `{}` is both an identifier and text",
                node.kind()
            );
            if ident == Ident::Value {
                values += 1;
            }
        }
        assert!(values > 0, "{language}: the sample has no value identifier");
    }
}

#[test]
fn the_sample_has_text_and_containers_and_a_doc_comment_is_text() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let sample = Sample::of(syntax);
        let nodes = sample.nodes();
        let text_kinds = syntax.text_kinds();
        assert!(
            nodes.iter().any(|node| text_kinds.contains(&node.kind())),
            "{language}: the sample has no comment or string"
        );
        let containers: Vec<_> = nodes
            .iter()
            .filter(|node| syntax.is_container(**node))
            .collect();
        assert!(
            !containers.is_empty(),
            "{language}: the sample has no container, so no folded view lists its methods"
        );
        for node in containers {
            assert!(
                syntax.block_kinds().contains(&node.kind()),
                "{language}: the container `{}` is not a block kind",
                node.kind()
            );
        }
        for node in &nodes {
            assert!(
                !syntax.is_doc(*node, &sample.source) || text_kinds.contains(&node.kind()),
                "{language}: the doc comment `{}` is not a text kind",
                node.kind()
            );
        }
    }
}
