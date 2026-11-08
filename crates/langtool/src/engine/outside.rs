//! The calls of one function that leave the scanned code, counted by where they go, for the
//! summary line of the call graph and of the outgoing view.

use crate::language::Origin;

/// External names named in a summary line before the rest is counted.
const NAMES_SHOWN: usize = 6;

/// The calls of one function that have no definition in the index, grouped by where they go.
#[derive(Default)]
pub struct Outside {
    std: usize,
    unresolved: usize,
    deps: Vec<String>,
    project: Vec<String>,
}

impl Outside {
    pub fn add(&mut self, name: &str, origin: &Origin) {
        match origin {
            Origin::Std => self.std += 1,
            Origin::Unknown => self.unresolved += 1,
            Origin::Dependency(package) => push_unique(&mut self.deps, package),
            Origin::Project => push_unique(&mut self.project, name),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.std == 0 && self.unresolved == 0 && self.deps.is_empty() && self.project.is_empty()
    }

    /// `2 std, 1 unresolved; deps: serde_json; project: helper`.
    pub fn render(&self) -> String {
        let mut parts = Vec::new();
        let counts: Vec<String> = [(self.std, "std"), (self.unresolved, "unresolved")]
            .iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, label)| format!("{count} {label}"))
            .collect();
        if !counts.is_empty() {
            parts.push(counts.join(", "));
        }
        if !self.deps.is_empty() {
            parts.push(format!("deps: {}", names(&self.deps)));
        }
        if !self.project.is_empty() {
            parts.push(format!("project: {}", names(&self.project)));
        }
        parts.join("; ")
    }
}

fn push_unique(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|known| known == name) {
        list.push(name.to_string());
    }
}

/// The first names of `list`, then how many more there are.
fn names(list: &[String]) -> String {
    let shown = list
        .iter()
        .take(NAMES_SHOWN)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    match list.len().saturating_sub(NAMES_SHOWN) {
        0 => shown,
        more => format!("{shown} +{more}"),
    }
}
