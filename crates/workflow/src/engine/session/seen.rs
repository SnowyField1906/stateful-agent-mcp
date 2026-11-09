//! What the agent has been shown of a file, kept in memory so that a patch quoting an older version
//! can be recognised and placed in the file as it is now (see `ops::rebase`).
//!
//! The store holds up to [`MAX_FILES`] files with [`MAX_VERSIONS`] versions each, newest first, and
//! about [`MAX_BYTES`] of text in all. It is never saved: a restarted server has seen nothing.

use std::collections::VecDeque;

const MAX_FILES: usize = 24;
const MAX_VERSIONS: usize = 3;
const MAX_BYTES: usize = 3 << 20;

/// Where a version came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// A read tool showed the file as it was on disk.
    Read,
    /// The text of a write before the formatter changed it.
    PreFormat,
}

/// One version of a file that the agent was shown.
#[derive(Clone, Debug)]
pub(crate) struct Sighting {
    pub text: String,
    pub origin: Origin,
    /// The last turn that had changed the file when this was recorded (`SS.TT`).
    pub last_turn: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Seen {
    /// The most recently noted file first, with its versions newest first.
    files: VecDeque<(String, Vec<Sighting>)>,
}

impl Seen {
    /// Record `sighting` as the newest version of `rel`. A version too big to be one of
    /// [`MAX_VERSIONS`] within the budget is not kept.
    pub(crate) fn note(&mut self, rel: &str, sighting: Sighting) {
        if sighting.text.len() > MAX_BYTES / MAX_VERSIONS {
            return;
        }
        let mut versions = match self.files.iter().position(|(path, _)| path == rel) {
            Some(at) => self.files.remove(at).map(|(_, v)| v).unwrap_or_default(),
            None => Vec::new(),
        };
        versions.retain(|kept| kept.text != sighting.text);
        versions.insert(0, sighting);
        versions.truncate(MAX_VERSIONS);
        self.files.push_front((rel.to_string(), versions));
        self.files.truncate(MAX_FILES);
        while self.files.len() > 1 && self.bytes() > MAX_BYTES {
            self.files.pop_back();
        }
    }

    /// The versions of `rel`, newest first.
    pub(crate) fn versions(&self, rel: &str) -> Vec<Sighting> {
        self.files
            .iter()
            .find(|(path, _)| path == rel)
            .map(|(_, versions)| versions.clone())
            .unwrap_or_default()
    }

    fn bytes(&self) -> usize {
        self.files
            .iter()
            .flat_map(|(_, versions)| versions)
            .map(|sighting| sighting.text.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Sighting {
        Sighting {
            text: text.to_string(),
            origin: Origin::Read,
            last_turn: None,
        }
    }

    #[test]
    fn versions_come_newest_first_and_a_repeat_moves_to_the_front() {
        let mut seen = Seen::default();
        seen.note("a.rs", read("one"));
        seen.note("a.rs", read("two"));
        seen.note("a.rs", read("one"));
        let texts: Vec<String> = seen.versions("a.rs").into_iter().map(|s| s.text).collect();
        assert_eq!(texts, ["one", "two"]);
        assert!(seen.versions("b.rs").is_empty());
    }

    #[test]
    fn old_versions_and_old_files_are_dropped() {
        let mut seen = Seen::default();
        for i in 0..5 {
            seen.note("a.rs", read(&format!("v{i}")));
        }
        assert_eq!(seen.versions("a.rs").len(), MAX_VERSIONS);
        assert_eq!(seen.versions("a.rs")[0].text, "v4");
        for i in 0..MAX_FILES + 1 {
            seen.note(&format!("f{i}.rs"), read("x"));
        }
        assert!(seen.versions("a.rs").is_empty());
        assert_eq!(seen.versions(&format!("f{MAX_FILES}.rs")).len(), 1);
    }

    #[test]
    fn a_text_over_the_budget_share_is_not_kept() {
        let mut seen = Seen::default();
        seen.note("big.rs", read(&"x".repeat(MAX_BYTES / MAX_VERSIONS + 1)));
        assert!(seen.versions("big.rs").is_empty());
    }
}
