//! Line diff producing [`PatchHunk`]s that reproduce `after` from `before` when applied in
//! order with [`super::patch::apply_hunks`].
//!
//! Lines keep their terminators, so differences in trailing newlines and line endings are
//! represented exactly.

use crate::common::LineNumber;

use super::PatchHunk;

/// Above this many LCS table cells, the changed middle becomes a single hunk.
pub(super) const LCS_CELL_LIMIT: usize = 4_000_000;

/// Hunks transforming `before` into `after`, with `lines` set to the 1-based line range in
/// `before`. Returns no hunks when the texts are equal.
pub fn compute_hunks(before: &str, after: &str) -> Vec<PatchHunk> {
    if before == after {
        return Vec::new();
    }
    if before.is_empty() {
        return vec![PatchHunk {
            start_line: Some(LineNumber::ONE),
            old_text: String::new(),
            new_text: after.to_string(),
            symbol: None,
        }];
    }
    if after.is_empty() {
        return vec![PatchHunk {
            start_line: Some(LineNumber::ONE),
            old_text: before.to_string(),
            new_text: String::new(),
            symbol: None,
        }];
    }
    let a: Vec<&str> = before.split_inclusive('\n').collect();
    let b: Vec<&str> = after.split_inclusive('\n').collect();

    let raw_regions = diff_regions(&a, &b);

    // Anchor pure insertions (`o0 == o1`) and pure deletions (`n0 == n1`) on an adjacent
    // unchanged line so both `old_text` and `new_text` are non-empty, and merge regions that
    // touch or overlap after anchoring.
    let mut regions: Vec<(usize, usize, usize, usize)> = Vec::with_capacity(raw_regions.len());
    for (mut o0, mut o1, mut n0, mut n1) in raw_regions {
        if o0 == o1 || n0 == n1 {
            if o0 > 0 && n0 > 0 {
                o0 -= 1;
                n0 -= 1;
            } else if o1 < a.len() && n1 < b.len() {
                o1 += 1;
                n1 += 1;
            }
        }
        if let Some(last) = regions.last_mut()
            && (last.1 >= o0 || last.3 >= n0)
        {
            last.1 = last.1.max(o1);
            last.3 = last.3.max(n1);
        } else {
            regions.push((o0, o1, n0, n1));
        }
    }

    let mut hunks = Vec::with_capacity(regions.len());
    for (o0, o1, n0, n1) in regions {
        hunks.push(PatchHunk {
            start_line: Some(LineNumber::from_zero_based(o0)),
            old_text: a[o0..o1].concat(),
            new_text: b[n0..n1].concat(),
            symbol: None,
        });
    }
    hunks
}

/// Changed line regions `(old_start, old_end, new_start, new_end)` between `before` and `after`:
/// 0-based, end exclusive, over lines that keep their terminators. A region holds no line that
/// both texts share and carries no context, so a pure insertion has `old_start == old_end`.
pub fn line_regions(before: &str, after: &str) -> Vec<(usize, usize, usize, usize)> {
    let a: Vec<&str> = before.split_inclusive('\n').collect();
    let b: Vec<&str> = after.split_inclusive('\n').collect();
    diff_regions(&a, &b)
}

/// [`line_regions`] over split lines: the shared leading and trailing lines are cut first, and the
/// middle is aligned by LCS, or is one region when the table would pass [`LCS_CELL_LIMIT`].
fn diff_regions(a: &[&str], b: &[&str]) -> Vec<(usize, usize, usize, usize)> {
    let (prefix, suffix) = affixes(a, b);
    let (a_mid, b_mid) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    if a_mid.len().saturating_mul(b_mid.len()) > LCS_CELL_LIMIT {
        return vec![(prefix, a.len() - suffix, prefix, b.len() - suffix)];
    }
    lcs_regions(a_mid, b_mid)
        .into_iter()
        .map(|(o0, o1, n0, n1)| (o0 + prefix, o1 + prefix, n0 + prefix, n1 + prefix))
        .collect()
}

/// Changed regions `(old_start, old_end, new_start, new_end)` between `a` and `b`.
pub(super) fn lcs_regions(a: &[&str], b: &[&str]) -> Vec<(usize, usize, usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let width = m + 1;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if a[i] == b[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }

    let mut regions = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut open: Option<(usize, usize)> = None;
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            if let Some((oi, oj)) = open.take() {
                regions.push((oi, i, oj, j));
            }
            i += 1;
            j += 1;
        } else {
            open.get_or_insert((i, j));
            if j >= m || (i < n && table[(i + 1) * width + j] >= table[i * width + j + 1]) {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    if let Some((oi, oj)) = open {
        regions.push((oi, n, oj, m));
    }
    regions
}

/// Lines `(added, removed)` by a hunk, ignoring every unchanged line inside it, also those between
/// two changed lines, so that the count equals the net diff the ledger records for the same edit.
pub fn hunk_stats(hunk: &PatchHunk) -> (usize, usize) {
    line_regions(&hunk.old_text, &hunk.new_text)
        .into_iter()
        .fold(
            (0, 0),
            |(added, removed), (old_start, old_end, new_start, new_end)| {
                (
                    added + (new_end - new_start),
                    removed + (old_end - old_start),
                )
            },
        )
}

/// Number of leading and trailing lines that `a` and `b` share.
pub(super) fn affixes(a: &[&str], b: &[&str]) -> (usize, usize) {
    let mut prefix = 0;
    while prefix < a.len() && prefix < b.len() && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < a.len() - prefix
        && suffix < b.len() - prefix
        && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix]
    {
        suffix += 1;
    }
    (prefix, suffix)
}

/// Unified-diff body of a hunk: unchanged lines start with a space, removed lines with `-`,
/// added lines with `+`. A last line without a newline is followed by git's marker.
pub fn hunk_body(hunk: &PatchHunk) -> String {
    let old: Vec<&str> = hunk.old_text.split_inclusive('\n').collect();
    let new: Vec<&str> = hunk.new_text.split_inclusive('\n').collect();
    let regions = if old.len().saturating_mul(new.len()) <= LCS_CELL_LIMIT {
        lcs_regions(&old, &new)
    } else {
        let (prefix, suffix) = affixes(&old, &new);
        vec![(prefix, old.len() - suffix, prefix, new.len() - suffix)]
    };
    let mut out = String::new();
    let mut at = 0;
    for (o0, o1, n0, n1) in regions {
        push_lines(&mut out, ' ', &old[at..o0]);
        push_lines(&mut out, '-', &old[o0..o1]);
        push_lines(&mut out, '+', &new[n0..n1]);
        at = o1;
    }
    push_lines(&mut out, ' ', &old[at..]);
    out
}

fn push_lines(out: &mut String, prefix: char, lines: &[&str]) {
    for line in lines {
        out.push(prefix);
        out.push_str(line.strip_suffix('\n').unwrap_or(line));
        out.push('\n');
        if !line.ends_with('\n') {
            out.push_str("\\ No newline at end of file\n");
        }
    }
}
