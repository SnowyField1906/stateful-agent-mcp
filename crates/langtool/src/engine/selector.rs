//! The text that names a symbol, accepted by every tool that takes one.

use super::scope::{Lineage, bare_name};

/// A symbol written as a name with optional qualifiers: `put`, `Store::put`, `Store.put` or
/// `east/Token/parse`. The same text is accepted by every tool that takes a symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolSelector<'a> {
    /// Enclosing definitions, outermost first. They may skip levels (`east::parse` reaches
    /// `east::Token::parse`).
    pub qualifiers: Vec<&'a str>,
    pub name: &'a str,
}

impl<'a> SymbolSelector<'a> {
    pub fn parse(raw: &'a str) -> Self {
        let trimmed = raw.trim().trim_start_matches(['/', ':']);
        let parts: Vec<&str> = trimmed
            .split("::")
            .flat_map(|part| part.split('/'))
            .flat_map(|part| part.split('.'))
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        match parts.split_last() {
            Some((&name, qualifiers)) => Self {
                qualifiers: qualifiers.to_vec(),
                name,
            },
            None => Self {
                qualifiers: Vec::new(),
                name: trimmed,
            },
        }
    }

    /// Whether the qualifiers appear, in order and ignoring case, among the names of the
    /// enclosing definitions `scope`. Generic parameters are ignored.
    pub fn matches_scope(&self, scope: &Lineage) -> bool {
        let mut wanted = self.qualifiers.iter();
        let mut next = wanted.next();
        for frame in scope.frames() {
            let Some(target) = next else { break };
            let matches_name = bare_name(&frame.name).eq_ignore_ascii_case(target);
            let matches_trait = frame
                .trait_name
                .as_deref()
                .is_some_and(|name| bare_name(name).eq_ignore_ascii_case(target));
            if matches_name || matches_trait {
                next = wanted.next();
            }
        }
        next.is_none()
    }
}
