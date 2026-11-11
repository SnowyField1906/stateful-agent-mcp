//! `Calls`: a call site names the same call as `call_name`, and the sample shows how calls are
//! written.

use stateful_agent_language::CallKind;

use super::support::{Sample, name, syntaxes};

#[test]
fn a_call_site_names_what_call_name_names() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let sample = Sample::of(syntax);
        for node in sample.nodes() {
            let Some(site) = syntax.call_site(node, &sample.source) else {
                continue;
            };
            assert_eq!(
                syntax.call_name(node, &sample.source).as_deref(),
                Some(site.name.as_str()),
                "{language}: `{}` is a call site of another name than its call name",
                sample.text(node)
            );
            assert!(
                !site.name.is_empty() && !site.name.contains([' ', '.', ':']),
                "{language}: the call name {:?} is not a plain name",
                site.name
            );
        }
    }
}

#[test]
fn a_language_whose_types_do_not_inherit_has_no_class() {
    for syntax in syntaxes() {
        if syntax.types_inherit_methods() {
            continue;
        }
        for kind in syntax.kinds() {
            assert!(
                !(kind.owner && kind.declares_type && !kind.interface),
                "{}: the kind `{}` is a class, but the language says its types do not inherit",
                name(syntax),
                kind.name
            );
        }
    }
}

#[test]
fn the_sample_shows_each_way_a_call_is_written() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let sample = Sample::of(syntax);
        let sites: Vec<_> = sample
            .nodes()
            .into_iter()
            .filter_map(|node| syntax.call_site(node, &sample.source))
            .collect();

        for site in &sites {
            if let CallKind::MethodOf(ty) | CallKind::Dyn(ty) | CallKind::Path(ty) = &site.kind {
                assert!(
                    !ty.is_empty() && !ty.contains(char::is_whitespace),
                    "{language}: the call `{}` names the type {ty:?}",
                    site.name
                );
            }
            assert!(
                site.root.as_ref().is_none_or(|root| !root.is_empty()),
                "{language}: the call `{}` has an empty root",
                site.name
            );
        }

        let has = |wanted: fn(&CallKind) -> bool| sites.iter().any(|site| wanted(&site.kind));
        assert!(
            has(|kind| *kind == CallKind::Free),
            "{language}: the sample has no call of a function"
        );
        assert!(
            has(|kind| *kind == CallKind::Own),
            "{language}: the sample has no call on `self`"
        );
        assert!(
            has(|kind| {
                matches!(
                    kind,
                    CallKind::Path(_) | CallKind::Method | CallKind::MethodOf(_) | CallKind::Dyn(_)
                )
            }),
            "{language}: the sample has no call through a path or on a value"
        );
    }
}
