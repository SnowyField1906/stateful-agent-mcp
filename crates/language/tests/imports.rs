//! `Imports`: the import statements of the sample bind names, and an alias is one of them.

use super::support::{Sample, name, syntaxes};

#[test]
fn import_statements_bind_names_and_no_other_node_does() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let sample = Sample::of(syntax);
        let kinds = syntax.import_kinds();
        let mut statements = 0;
        for node in sample.nodes() {
            let imports = syntax.imports(node, &sample.source);
            if !kinds.contains(&node.kind()) {
                assert!(
                    imports.is_empty(),
                    "{language}: `{}` binds names but is not an import kind",
                    node.kind()
                );
                continue;
            }
            statements += 1;
            let text = sample.text(node);
            assert!(!imports.is_empty(), "{language}: `{text}` binds no name");
            assert!(
                syntax.alias(node, &sample.source).is_none(),
                "{language}: `{text}` is an alias, but only its parts are"
            );
            for import in &imports {
                assert!(
                    !import.local.is_empty() && !import.local.contains(char::is_whitespace),
                    "{language}: `{text}` binds the name {:?}",
                    import.local
                );
                assert!(
                    import.relative || !import.path.is_empty(),
                    "{language}: `{text}` binds a name without a path"
                );
                assert!(
                    import
                        .path
                        .iter()
                        .all(|part| !part.is_empty() && !part.contains(char::is_whitespace)),
                    "{language}: `{text}` has the path {:?}",
                    import.path
                );
            }
        }
        assert!(
            kinds.is_empty() || statements > 0,
            "{language}: the sample has no import"
        );
    }
}

#[test]
fn an_alias_is_a_name_its_statement_binds() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let sample = Sample::of(syntax);
        let mut aliases = 0;
        for node in sample.nodes() {
            let Some(alias) = syntax.alias(node, &sample.source) else {
                continue;
            };
            aliases += 1;
            assert!(
                !alias.alias.is_empty() && !alias.original.is_empty(),
                "{language}: `{}` is an alias of nothing",
                sample.text(node)
            );
            let statement = std::iter::successors(node.parent(), |node| node.parent())
                .find(|node| syntax.import_kinds().contains(&node.kind()));
            let bound = statement.is_some_and(|statement| {
                syntax
                    .imports(statement, &sample.source)
                    .iter()
                    .any(|import| import.local == alias.alias)
            });
            assert!(
                bound,
                "{language}: no import statement binds the alias `{}`",
                alias.alias
            );
        }
        assert!(
            syntax.import_kinds().is_empty() || aliases > 0,
            "{language}: the sample has no aliased import"
        );
    }
}

#[test]
fn module_entry_stems_are_file_stems() {
    for syntax in syntaxes() {
        for stem in syntax.module_entry_stems() {
            assert!(
                !stem.is_empty() && !stem.contains(['.', '/']),
                "{}: `{stem}` is not a file stem",
                name(syntax)
            );
        }
    }
}
