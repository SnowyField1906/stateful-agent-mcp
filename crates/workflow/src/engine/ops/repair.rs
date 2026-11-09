//! Placing a hunk whose `old_text` is not in the file byte for byte but is provably the same text.
//!
//! Two proofs are accepted: the quote's lines equal the file's lines up to trailing blanks, one
//! indentation change and, on lines with no string or comment, runs of blanks inside the line
//! (`spacing`), or the file already holds the hunk's `new_text` (`already applied`). They are looked
//! for only after the exact match failed, so a hunk that matches costs nothing here.

use std::ops::Range;

use crate::common::align::{self, Compare};
use crate::language::{line_span, parse, registry};

use super::diff::affixes;

/// A place where a quote fits the file's lines up to whitespace.
pub(super) struct Window {
    /// Byte offset where the replaced text starts: the start of its first line.
    pub at: usize,
    /// The file's text between the two, which the replacement takes the place of.
    pub old: String,
    /// The hunk's `new_text`, written in the file's indentation.
    pub new: String,
    /// How the quote fits, for the summary line: `spacing: indent +4 spaces`.
    pub how: String,
}

/// 1-based `(first, last)` lines of every string or comment of `text` that spans several lines.
/// Whitespace inside one is content, so a quote that reaches into one is never matched loosely.
pub(super) fn text_spans(path: &str, text: &str) -> Vec<(usize, usize)> {
    text_nodes(path, text)
        .unwrap_or_default()
        .into_iter()
        .filter(|&(first, last)| last > first)
        .collect()
}

/// 1-based `(first, last)` lines of every string or comment of `text`; `None` when the file's
/// language has no grammar or the file does not parse.
fn text_nodes(path: &str, text: &str) -> Option<Vec<(usize, usize)>> {
    let syntax = registry::for_path(path)?;
    let tree = parse(syntax, text)?;
    let kinds = syntax.text_kinds();
    let mut nodes = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        let is_text = kinds.contains(&node.kind());
        if is_text {
            nodes.push(line_span(node));
        }
        if !is_text && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                continue 'walk;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    Some(nodes)
}

/// The places where the lines of `old` are lines of `text` up to trailing blanks and one
/// indentation change, each with `new` rewritten for the file's indentation. A place inside
/// `scope` only, when there is one, and never one that reaches into a multi-line string or
/// comment (`spans`). The quote has to be whole lines: a first or last line that is only part of
/// a file line does not fit.
pub(super) fn spacing_windows(
    text: &str,
    old: &str,
    new: &str,
    scope: Option<&Range<usize>>,
    spans: &[(usize, usize)],
) -> Vec<Window> {
    let file: Vec<&str> = text.split_inclusive('\n').collect();
    let quote: Vec<&str> = old.split_inclusive('\n').collect();
    if quote.is_empty() || quote.len() > file.len() {
        return Vec::new();
    }
    let file_bodies: Vec<&str> = file.iter().map(|line| align::body(line)).collect();
    let quote_bodies: Vec<&str> = quote.iter().map(|line| align::body(line)).collect();
    let starts = line_starts(&file);
    let ends_with_newline = old.ends_with('\n');

    let mut windows = Vec::new();
    for (line, fit) in align::find_fits(&quote_bodies, &file_bodies, Compare::Trimmed) {
        let last = line + quote.len() - 1;
        let at = starts[line];
        let end = if ends_with_newline {
            starts[last] + file[last].len()
        } else {
            starts[last] + file_bodies[last].len()
        };
        if scope.is_some_and(|s| at < s.start || end > s.end) {
            continue;
        }
        if spans
            .iter()
            .any(|&(first, final_line)| first <= last + 1 && line < final_line)
        {
            continue;
        }
        let Some(replacement) = align::reindent(new, fit.shift.as_ref()) else {
            continue;
        };
        windows.push(Window {
            at,
            old: text[at..end].to_string(),
            new: replacement,
            how: describe(&fit, false),
        });
    }
    windows
}

/// The places where the lines of `old` are lines of `text` up to what [`spacing_windows`] allows
/// and runs of blanks inside a line. Only the lines the hunk changes are replaced, so a context
/// line keeps the file's own spacing. A line that differs from its quote in inner blanks must hold
/// no string or comment, whose blanks are content, and the file must have a grammar. The same
/// bounds apply as for [`spacing_windows`].
pub(super) fn squeezed_windows(
    path: &str,
    text: &str,
    (old, new): (&str, &str),
    scope: Option<&Range<usize>>,
    spans: &[(usize, usize)],
) -> Vec<Window> {
    if old.ends_with('\n') != new.ends_with('\n') {
        return Vec::new();
    }
    let Some(nodes) = text_nodes(path, text) else {
        return Vec::new();
    };
    let file: Vec<&str> = text.split_inclusive('\n').collect();
    let quote: Vec<&str> = old.split_inclusive('\n').collect();
    let after: Vec<&str> = new.split_inclusive('\n').collect();
    if quote.is_empty() || quote.len() > file.len() {
        return Vec::new();
    }
    let (prefix, suffix) = affixes(&quote, &after);
    let came = after[prefix..after.len() - suffix].concat();
    let gone = quote.len() - prefix - suffix;
    if gone == 0 && came.is_empty() {
        return Vec::new();
    }
    let file_bodies: Vec<&str> = file.iter().map(|line| align::body(line)).collect();
    let quote_bodies: Vec<&str> = quote.iter().map(|line| align::body(line)).collect();
    let starts = line_starts(&file);
    let unterminated = !text.is_empty() && !text.ends_with('\n');

    let mut windows = Vec::new();
    for (line, fit) in align::find_fits(&quote_bodies, &file_bodies, Compare::Squeezed) {
        let last = line + quote.len() - 1;
        // The file lines the hunk replaces: `first..stop`.
        let (first, stop) = (line + prefix, last + 1 - suffix);
        let at = starts.get(first).copied().unwrap_or(text.len());
        let end = match gone {
            0 => at,
            _ if suffix == 0 && !old.ends_with('\n') => {
                starts[stop - 1] + file_bodies[stop - 1].len()
            }
            _ => starts[stop - 1] + file[stop - 1].len(),
        };
        if gone == 0 && at == text.len() && unterminated {
            continue;
        }
        if scope.is_some_and(|s| at < s.start || end > s.end) {
            continue;
        }
        if spans
            .iter()
            .any(|&(first_line, final_line)| first_line <= last + 1 && line < final_line)
        {
            continue;
        }
        // The lines whose inner blanks differ. At least one, or the plain spacing rule would have
        // placed the quote; none of them may hold part of a string or comment.
        let loose: Vec<usize> = (0..quote.len())
            .filter(|&i| quote_bodies[i].trim() != file_bodies[line + i].trim())
            .collect();
        let on_text = |n: usize| nodes.iter().any(|&(from, to)| from <= n && n <= to);
        if loose.is_empty() || loose.iter().any(|&i| on_text(line + i + 1)) {
            continue;
        }
        let Some(replacement) = align::reindent(&came, fit.shift.as_ref()) else {
            continue;
        };
        windows.push(Window {
            at,
            old: text[at..end].to_string(),
            new: replacement,
            how: describe(&fit, true),
        });
    }
    windows
}

/// The byte offset where each of `file`'s lines starts.
fn line_starts(file: &[&str]) -> Vec<usize> {
    let mut starts = Vec::with_capacity(file.len());
    let mut offset = 0;
    for line in file {
        starts.push(offset);
        offset += line.len();
    }
    starts
}

/// `spacing: indent +4 spaces, inner whitespace`; `inner` is true for a quote that needed its
/// inner blanks squeezed.
fn describe(fit: &align::Fit, inner: bool) -> String {
    let mut parts: Vec<String> = fit.shift.iter().map(align::Shift::describe).collect();
    if inner {
        parts.push("inner whitespace".to_string());
    }
    if parts.is_empty() {
        "spacing: trailing whitespace".to_string()
    } else {
        format!("spacing: {}", parts.join(", "))
    }
}

/// The byte offset where `text` already holds `new`, when it holds it exactly once and `new` is
/// big enough (three lines or 80 characters) to be no coincidence. Whitespace may differ as in
/// [`spacing_windows`]. Short replacements prove nothing, so they are never taken for applied.
pub(super) fn already_applied(
    text: &str,
    new: &str,
    scope: Option<&Range<usize>>,
    spans: &[(usize, usize)],
) -> Option<usize> {
    if new.lines().count() < 3 && new.trim().chars().count() < 80 {
        return None;
    }
    let exact: Vec<usize> = text
        .match_indices(new)
        .map(|(i, _)| i)
        .filter(|&i| scope.is_none_or(|s| i >= s.start && i + new.len() <= s.end))
        .collect();
    match exact.as_slice() {
        [only] => return Some(*only),
        [] => {}
        _ => return None,
    }
    match spacing_windows(text, new, new, scope, spans).as_slice() {
        [only] => Some(only.at),
        _ => None,
    }
}
