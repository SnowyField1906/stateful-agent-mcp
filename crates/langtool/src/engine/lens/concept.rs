//! Words to definitions, without embeddings: the subwords of a query are matched against the
//! subwords of the names, signatures and documentation of the definitions, and of the paths and
//! documentation of their files. A word that few definitions match counts for more.

use std::collections::HashMap;

use super::codebase::Codebase;
use super::words::subwords;

const NAME_WEIGHT: f64 = 3.0;
const SIGNATURE_WEIGHT: f64 = 1.5;
const DOC_WEIGHT: f64 = 1.0;
const PATH_WEIGHT: f64 = 1.0;
const FILE_DOC_WEIGHT: f64 = 0.5;
/// A query word that starts, or is started by, a word of the index.
const PREFIX_MATCH: f64 = 0.6;
/// A query word one edit away from a word of the index.
const NEAR_MATCH: f64 = 0.5;
const MIN_PREFIX_LEN: usize = 4;
const MIN_NEAR_LEN: usize = 5;
/// A seed must score at least this share of the best one.
const SEED_SHARE: f64 = 0.4;

/// A definition that the words point at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub node: usize,
    pub score: f64,
}

/// The distinct subwords of a query, in the order written.
pub fn query_words(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in subwords(query) {
        if !out.contains(&word) {
            out.push(word);
        }
    }
    out
}

/// The best `limit` definitions for `words`, the best first, without those that score under
/// 40% of the best. Ties go to the more used definition, then to the earlier one.
pub fn lookup(codebase: &Codebase, words: &[String], limit: usize) -> Vec<Hit> {
    if words.is_empty() || limit == 0 {
        return Vec::new();
    }
    let file_bags: Vec<HashMap<String, f64>> = (0..codebase.files.len())
        .map(|file| {
            let mut bag = HashMap::new();
            add(&mut bag, codebase.label(file), PATH_WEIGHT);
            if let Some(doc) = &codebase.facts(file).doc {
                add(&mut bag, doc, FILE_DOC_WEIGHT);
            }
            bag
        })
        .collect();

    // Per matching definition: the strength of each word.
    let mut matched: Vec<(usize, Vec<f64>)> = Vec::new();
    let mut spread = vec![0usize; words.len()];
    let mut listed = 0usize;
    for node in 0..codebase.graph.len() {
        if !codebase.listed(node) {
            continue;
        }
        listed += 1;
        let symbol = &codebase.def(node).symbol;
        let mut bag = HashMap::new();
        add(&mut bag, &symbol.name, NAME_WEIGHT);
        add(&mut bag, &symbol.signature, SIGNATURE_WEIGHT);
        add(&mut bag, &symbol.doc, DOC_WEIGHT);
        let file_bag = &file_bags[codebase.file_of(node)];
        let strengths: Vec<f64> = words
            .iter()
            .map(|word| strength(word, &bag).max(strength(word, file_bag)))
            .collect();
        if strengths.iter().all(|&s| s <= 0.0) {
            continue;
        }
        for (count, &s) in spread.iter_mut().zip(&strengths) {
            if s > 0.0 {
                *count += 1;
            }
        }
        matched.push((node, strengths));
    }

    let idf: Vec<f64> = spread
        .iter()
        .map(|&df| (1.0 + listed as f64 / (1.0 + df as f64)).ln())
        .collect();
    let mut hits: Vec<Hit> = matched
        .into_iter()
        .map(|(node, strengths)| {
            let covered = strengths.iter().filter(|&&s| s > 0.0).count();
            let sum: f64 = strengths.iter().zip(&idf).map(|(s, i)| s * i).sum();
            let coverage = (covered as f64 / words.len() as f64).sqrt();
            Hit {
                node,
                score: sum * coverage,
            }
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(
                codebase
                    .graph
                    .load(b.node)
                    .ca
                    .total_cmp(&codebase.graph.load(a.node).ca),
            )
            .then(a.node.cmp(&b.node))
    });
    let best = hits.first().map_or(0.0, |hit| hit.score);
    hits.retain(|hit| hit.score >= best * SEED_SHARE);
    hits.truncate(limit);
    hits
}

/// Put the subwords of `text` in `bag` with `weight`, keeping the larger weight of a word that
/// is already there.
fn add(bag: &mut HashMap<String, f64>, text: &str, weight: f64) {
    for word in subwords(text) {
        let slot = bag.entry(word).or_insert(0.0);
        if weight > *slot {
            *slot = weight;
        }
    }
}

/// How strongly `bag` holds `query`: the weight of the word itself, or a part of the weight of a
/// word that starts alike or is one edit away.
fn strength(query: &str, bag: &HashMap<String, f64>) -> f64 {
    let mut best = bag.get(query).copied().unwrap_or(0.0);
    let query_len = query.chars().count();
    for (word, weight) in bag {
        if word == query {
            continue;
        }
        let word_len = word.chars().count();
        let factor = if query_len >= MIN_PREFIX_LEN
            && word_len >= MIN_PREFIX_LEN
            && (word.starts_with(query) || query.starts_with(word.as_str()))
        {
            PREFIX_MATCH
        } else if query_len >= MIN_NEAR_LEN && word_len >= MIN_NEAR_LEN && within_one(query, word) {
            NEAR_MATCH
        } else {
            continue;
        };
        best = best.max(factor * weight);
    }
    best
}

/// Whether the words differ by at most one insertion, deletion or substitution.
fn within_one(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        match a.len().cmp(&b.len()) {
            std::cmp::Ordering::Greater => i += 1,
            std::cmp::Ordering::Less => j += 1,
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    edits + (a.len() - i) + (b.len() - j) <= 1
}
