//! Small, bounded name matching for helpful miss suggestions across Tree-sitter tools.

/// Return a bounded edit-distance score for a plausible typo. Adjacent transpositions count as a
/// single edit, matching a common identifier typo pattern.
pub fn similarity_score(query: &str, candidate: &str) -> Option<usize> {
    let a: Vec<char> = query.to_lowercase().chars().collect();
    let b: Vec<char> = candidate.to_lowercase().chars().collect();
    if a.len() < 3 || a.len() > 64 || b.len() < 3 || b.len() > 64 || a == b {
        return None;
    }
    let limit = (a.len().max(b.len()) / 4).max(1);
    if a.len().abs_diff(b.len()) > limit {
        return None;
    }

    // Optimal-string-alignment distance, banded to the maximum useful edit distance.
    let infinity = limit + 1;
    let mut before_previous = vec![infinity; b.len() + 1];
    let mut previous = vec![infinity; b.len() + 1];
    for (j, cell) in previous.iter_mut().enumerate().take(limit.min(b.len()) + 1) {
        *cell = j;
    }
    let mut current = vec![infinity; b.len() + 1];
    for (i, left) in a.iter().enumerate() {
        let row = i + 1;
        current.fill(infinity);
        if row <= limit {
            current[0] = row;
        }
        let first = row.saturating_sub(limit).max(1);
        let last = (row + limit).min(b.len());
        let mut row_min = infinity;
        for col in first..=last {
            let right = &b[col - 1];
            let mut distance = (previous[col - 1] + usize::from(left != right))
                .min(previous[col] + 1)
                .min(current[col - 1] + 1);
            if row > 1 && col > 1 && a[row - 1] == b[col - 2] && a[row - 2] == b[col - 1] {
                distance = distance.min(before_previous[col - 2] + 1);
            }
            current[col] = distance;
            row_min = row_min.min(distance);
        }
        if row_min > limit {
            return None;
        }
        std::mem::swap(&mut before_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    (previous[b.len()] <= limit).then_some(previous[b.len()])
}

/// Rank unique symbol names by substring match first, then bounded edit distance.
pub fn ranked_names(
    names: impl IntoIterator<Item = String>,
    query: &str,
    limit: usize,
    allow_substrings: bool,
) -> Vec<(usize, String)> {
    let needle = query.to_lowercase();
    let mut ranked: Vec<(usize, String)> = names
        .into_iter()
        .filter_map(|name| {
            let lower = name.to_lowercase();
            let score = if allow_substrings
                && (lower.contains(&needle) || (lower.len() >= 3 && needle.contains(&lower)))
            {
                Some(0)
            } else {
                similarity_score(&needle, &name)
            }?;
            Some((score, name))
        })
        .collect();
    ranked.sort();
    ranked.dedup_by(|left, right| left.1 == right.1);
    ranked.truncate(limit);
    ranked
}
