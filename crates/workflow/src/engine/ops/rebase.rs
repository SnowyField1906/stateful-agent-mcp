//! Placing a hunk whose `old_text` quotes the file as the agent saw it, not as it is now.
//!
//! The lines a hunk changes are what is left of `old_text` once the lines it shares with `new_text`
//! at its start and at its end are taken off. Those lines are binding; the lines around them are
//! advisory. Two ways locate the changed lines in the file, and both replace only them, so a
//! context line that differs in the file stays as it is:
//!
//! - by version: `old_text` is verbatim in a version of the file that the agent was shown (see
//!   `session::seen`, and the turns of the ledger): the only place it occurs there, or the one
//!   nearest to the hunk's `start_line`. The lines that differ between that version and the file
//!   carry the position over, provided none of them touches or abuts the changed lines.
//! - by search: the changed lines occur once in the file (or one place has more matching context
//!   than the others) and are long enough to be no coincidence.
//!
//! Either way the lines found in the file are compared with the hunk's changed lines before
//! anything is replaced. A pure insertion has no changed old line, so only a version places it.

use std::cell::OnceCell;
use std::ops::Range;

use crate::common::align;

use super::diff::{LCS_CELL_LIMIT, affixes, lcs_regions};
use super::repair::Window;

/// A version of a file that the agent was shown, and who changed the file since.
#[derive(Clone, Debug)]
pub struct Version {
    pub text: String,
    /// `formatter`, `outside` or `your turn 02.03`.
    pub cause: String,
}

/// The versions of a file that a patch may quote, loaded when a hunk needs them: a patch whose
/// hunks all match never reads them.
pub struct History<'a> {
    load: &'a dyn Fn() -> Vec<Version>,
    loaded: OnceCell<Vec<Version>>,
}

impl<'a> History<'a> {
    pub fn new(load: &'a dyn Fn() -> Vec<Version>) -> Self {
        Self {
            load,
            loaded: OnceCell::new(),
        }
    }

    pub(super) fn versions(&self) -> &[Version] {
        self.loaded.get_or_init(|| (self.load)())
    }
}

/// What a hunk changes: the lines of its quote, how many of them lead and trail the changed ones,
/// and the lines that take the changed ones' place.
struct Cut<'a> {
    quote: Vec<&'a str>,
    prefix: usize,
    suffix: usize,
    came: Vec<&'a str>,
}

impl<'a> Cut<'a> {
    fn of(old: &'a str, new: &'a str) -> Option<Self> {
        if old.ends_with('\n') != new.ends_with('\n') {
            return None;
        }
        let quote: Vec<&str> = old.split_inclusive('\n').collect();
        let after: Vec<&str> = new.split_inclusive('\n').collect();
        let (prefix, suffix) = affixes(&quote, &after);
        let came = after[prefix..after.len() - suffix].to_vec();
        let gone = quote.len() - prefix - suffix;
        (gone + came.len() > 0).then_some(Self {
            quote,
            prefix,
            suffix,
            came,
        })
    }

    /// The quote's changed lines.
    fn gone(&self) -> &[&'a str] {
        &self.quote[self.prefix..self.quote.len() - self.suffix]
    }
}

/// The place in `text` where the changed lines of the hunk `old` -> `new` are, and the text that
/// replaces them. Inside `scope` only, when there is one. `hint` is the hunk's `start_line`, which
/// picks among the places `old` has in a version.
pub(super) fn rebase(
    text: &str,
    (old, new): (&str, &str),
    (scope, hint): (Option<&Range<usize>>, Option<usize>),
    versions: &[Version],
) -> Option<Window> {
    let cut = Cut::of(old, new)?;
    let file: Vec<&str> = text.split_inclusive('\n').collect();
    by_version(text, &file, (old, hint), &cut, versions)
        .or_else(|| by_search(text, &file, &cut))
        .filter(|w| scope.is_none_or(|s| w.at >= s.start && w.at + w.old.len() <= s.end))
}

/// Whether `old` is verbatim in a version the agent was shown that the file has changed since: who
/// changed it (`outside`, `formatter`, `your turn 02.03`) and which lines of the file differ from
/// that version. A hunk that could not be placed reports its quote as stale with this.
pub(super) fn stale_cause(
    text: &str,
    old: &str,
    hint: Option<usize>,
    versions: &[Version],
) -> Option<(String, String)> {
    let version = versions
        .iter()
        .find(|v| line_of(&v.text, old, hint).is_some())?;
    let then: Vec<&str> = version.text.split_inclusive('\n').collect();
    let file: Vec<&str> = text.split_inclusive('\n').collect();
    if then.len().saturating_mul(file.len()) > LCS_CELL_LIMIT {
        return None;
    }
    let regions = lcs_regions(&then, &file);
    if regions.is_empty() {
        return None;
    }
    Some((version.cause.clone(), changed_lines(&regions)))
}

/// Locate the changed lines through the newest version that holds `old` verbatim, once or, when
/// `hint` is the line it starts at, nearer to the hint than anywhere else.
fn by_version(
    text: &str,
    file: &[&str],
    (old, hint): (&str, Option<usize>),
    cut: &Cut<'_>,
    versions: &[Version],
) -> Option<Window> {
    let (version, line) = versions
        .iter()
        .find_map(|v| line_of(&v.text, old, hint).map(|line| (v, line)))?;
    let then: Vec<&str> = version.text.split_inclusive('\n').collect();
    if then.len().saturating_mul(file.len()) > LCS_CELL_LIMIT {
        return None;
    }
    let regions = lcs_regions(&then, file);
    let gone = cut.gone();
    let first = line + cut.prefix;
    // The lines of the version that no region may touch: the changed lines, or for an insertion
    // the lines on both sides of it.
    let (low, high) = if gone.is_empty() {
        (first.saturating_sub(1), (first + 1).min(then.len()))
    } else {
        (first, first + gone.len())
    };
    if regions
        .iter()
        .any(|&(o0, o1, _, _)| o0 <= high && o1 >= low)
    {
        return None;
    }
    let shift: isize = regions
        .iter()
        .filter(|region| region.1 < low)
        .map(|&(o0, o1, n0, n1)| (n1 - n0) as isize - (o1 - o0) as isize)
        .sum();
    let start = first.checked_add_signed(shift)?;
    if start + gone.len() > file.len() {
        return None;
    }
    let same = |a: &str, b: &str| align::body(a) == align::body(b);
    if !gone.iter().zip(&file[start..]).all(|(g, f)| same(g, f)) {
        return None;
    }
    if gone.is_empty() {
        let above = if first == 0 {
            start == 0
        } else {
            start > 0 && same(file[start - 1], then[first - 1])
        };
        let below = if first == then.len() {
            start == file.len()
        } else {
            start < file.len() && same(file[start], then[first])
        };
        if !(above && below) {
            return None;
        }
    }
    let how = format!(
        "rebased over {} (lines {} changed since you saw the file)",
        version.cause,
        changed_lines(&regions)
    );
    window(text, file, start, cut, how)
}

/// Locate the changed lines by themselves: the one place they occur, or the one with the most
/// matching context lines.
fn by_search(text: &str, file: &[&str], cut: &Cut<'_>) -> Option<Window> {
    let gone = cut.gone();
    let need = if gone.len() == 1 { 24 } else { 16 };
    let solid = gone
        .iter()
        .any(|line| line.chars().filter(|c| !c.is_whitespace()).count() >= need);
    if !solid || gone.len() > file.len() {
        return None;
    }
    let same = |a: &str, b: &str| align::body(a).trim_end() == align::body(b).trim_end();
    let found: Vec<usize> = (0..=file.len() - gone.len())
        .filter(|&m| gone.iter().zip(&file[m..]).all(|(g, f)| same(g, f)))
        .collect();
    let below = &cut.quote[cut.quote.len() - cut.suffix..];
    let score = |m: usize| -> usize {
        let above = (0..cut.prefix)
            .filter(|&i| {
                m.checked_sub(i + 1)
                    .is_some_and(|l| same(file[l], cut.quote[cut.prefix - 1 - i]))
            })
            .count();
        let after = below
            .iter()
            .enumerate()
            .filter(|&(i, q)| file.get(m + gone.len() + i).is_some_and(|f| same(f, q)))
            .count();
        above + after
    };
    let mut scored: Vec<(usize, usize)> = found.iter().map(|&m| (score(m), m)).collect();
    scored.sort_unstable_by(|a, b| b.cmp(a));
    let (best, start) = *scored.first()?;
    if scored.get(1).is_some_and(|&(next, _)| next >= best) {
        return None;
    }
    let above = (0..cut.prefix).find_map(|i| {
        let line = (start + i).checked_sub(cut.prefix)?;
        (!same(file.get(line)?, cut.quote[i])).then_some(line + 1)
    });
    let differs = above.or_else(|| {
        (0..cut.suffix).find_map(|i| {
            let line = start + gone.len() + i;
            (!same(file.get(line)?, below[i])).then_some(line + 1)
        })
    });
    let how = match differs {
        Some(line) => format!("changed lines kept, context differs at {line}"),
        None => "changed lines kept, context differs".to_string(),
    };
    window(text, file, start, cut, how)
}

/// The replacement of the `gone` lines found at line `start` (0-based) of the file, as a window.
fn window(text: &str, file: &[&str], start: usize, cut: &Cut<'_>, how: String) -> Option<Window> {
    let gone = cut.gone().len();
    let at: usize = file[..start].iter().map(|line| line.len()).sum();
    let end = at
        + file[start..start + gone]
            .iter()
            .map(|line| line.len())
            .sum::<usize>();
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut new = String::new();
    for line in &cut.came {
        new.push_str(align::body(line));
        new.push_str(eol);
    }
    let unterminated = !text.is_empty() && !text.ends_with('\n');
    if gone == 0 && at == text.len() && unterminated {
        return None;
    }
    if gone > 0 && end == text.len() && unterminated && !new.is_empty() {
        new.truncate(new.len() - eol.len());
    }
    Some(Window {
        at,
        old: text[at..end].to_string(),
        new,
        how,
    })
}

/// The 0-based line where `quote` starts in `text` as whole lines: the only place, or with a
/// `hint` (1-based) the one nearest to it when no other is as near.
fn line_of(text: &str, quote: &str, hint: Option<usize>) -> Option<usize> {
    let mut found: Vec<usize> = Vec::new();
    for (at, _) in text.match_indices(quote) {
        let end = at + quote.len();
        let starts = at == 0 || text.as_bytes()[at - 1] == b'\n';
        let ends = quote.ends_with('\n')
            || end == text.len()
            || text[end..].starts_with('\n')
            || text[end..].starts_with("\r\n");
        if starts && ends {
            found.push(text[..at].bytes().filter(|&b| b == b'\n').count());
        }
    }
    if let [only] = found.as_slice() {
        return Some(*only);
    }
    let hint = hint.filter(|_| found.len() > 1)?;
    let apart = |line: usize| (line + 1).abs_diff(hint);
    found.sort_unstable_by_key(|&line| apart(line));
    (apart(found[0]) < apart(found[1])).then_some(found[0])
}

/// The lines of the file that differ from the version, as 1-based ranges.
fn changed_lines(regions: &[(usize, usize, usize, usize)]) -> String {
    let shown: Vec<String> = regions
        .iter()
        .take(3)
        .map(|&(_, _, n0, n1)| {
            if n1 > n0 + 1 {
                format!("{}..{}", n0 + 1, n1)
            } else {
                (n0 + 1).to_string()
            }
        })
        .collect();
    match regions.len().saturating_sub(3) {
        0 => shown.join(", "),
        more => format!("{}, and {more} more", shown.join(", ")),
    }
}
