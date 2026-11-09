//! Words in names: the subwords of an identifier, and the few that tell a file from the others.

use std::collections::{BTreeMap, HashMap, HashSet};

/// A subword used by this share of the files, and by at least `MIN_COMMON_FILES` of them, says
/// nothing about any one of them.
const COMMON_SHARE: usize = 3;
const COMMON_OUT_OF: usize = 10;
const MIN_COMMON_FILES: usize = 4;

/// The lower-case subwords of `name` with three characters or more, split at `_`, at a lower case
/// letter or digit followed by a capital, and before the last capital of a run of capitals:
/// `parse_hunk` and `ParseHunk` give `parse` and `hunk`, `HTTPServer` gives `http` and `server`.
pub fn subwords(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for (at, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            flush(&mut current, &mut words);
            continue;
        }
        let starts_word = c.is_uppercase()
            && at > 0
            && chars[at - 1].is_alphanumeric()
            && (chars[at - 1].is_lowercase()
                || chars[at - 1].is_numeric()
                || chars.get(at + 1).is_some_and(|next| next.is_lowercase()));
        if starts_word {
            flush(&mut current, &mut words);
        }
        current.extend(c.to_lowercase());
    }
    flush(&mut current, &mut words);
    words
}

fn flush(current: &mut String, words: &mut Vec<String>) {
    if current.chars().count() >= 3 {
        words.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

/// How widely each subword is used among the files of a survey.
pub struct Words {
    files: usize,
    /// In how many files each subword occurs.
    spread: HashMap<String, usize>,
}

impl Words {
    /// The statistics of a survey, given as the names each file defines.
    pub fn new(per_file: &[Vec<String>]) -> Self {
        let mut spread: HashMap<String, usize> = HashMap::new();
        for names in per_file {
            let seen: HashSet<String> = names.iter().flat_map(|name| subwords(name)).collect();
            for word in seen {
                *spread.entry(word).or_default() += 1;
            }
        }
        Self {
            files: per_file.len(),
            spread,
        }
    }

    fn common(&self, word: &str) -> bool {
        let count = self.spread.get(word).copied().unwrap_or(0);
        count >= MIN_COMMON_FILES && count * COMMON_OUT_OF > self.files * COMMON_SHARE
    }

    /// The `n` subwords of `names` that tell them apart best: how often the name uses the word
    /// times how rare it is among the files, without the words that over 30% of the files use.
    pub fn top(&self, names: &[String], n: usize) -> Vec<String> {
        let mut frequency: BTreeMap<String, usize> = BTreeMap::new();
        for name in names {
            for word in subwords(name) {
                *frequency.entry(word).or_default() += 1;
            }
        }
        let mut scored: Vec<(f64, String)> = frequency
            .into_iter()
            .filter(|(word, _)| !self.common(word))
            .map(|(word, count)| {
                let spread = self.spread.get(&word).copied().unwrap_or(0) as f64;
                let rarity = ((1.0 + self.files as f64) / (1.0 + spread)).ln() + 1.0;
                (count as f64 * rarity, word)
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.into_iter().take(n).map(|(_, word)| word).collect()
    }
}
