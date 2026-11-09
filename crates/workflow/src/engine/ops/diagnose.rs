//! Saying why a hunk could not be placed.
//!
//! A hunk that no rung of the repair ladder placed is classified from the texts themselves, and the
//! report shows the lines that differ so that the next attempt can be right. The classes are
//! `stale` (the quote is a version the agent was shown, and the file changed since), `edit`, `gap`,
//! `extra` and `order` (read off the line alignment, see `common::align`), `spacing` (the lines match
//! up to whitespace that may not be touched), `ambiguous`, `overlap` (the quote holds lines that an
//! earlier hunk of the same call rewrote), `scope`, `elision` and `misquote` (no close window).

use std::cmp::Reverse;

use crate::common::align::{self, Pair, Tag};
use crate::langtool::engine::symbols::{self, Symbol};
use crate::language::registry;

use super::rebase::{self, Version};

/// Rows shown for lines that differ before the rest is counted.
const SHOWN_ROWS: usize = 6;
/// Lines of the current window that are printed to be quoted again.
const SHOWN_WINDOW: usize = 40;
/// Longest line printed, in characters.
const CUT: usize = 100;

/// Why one hunk was not placed.
#[derive(Debug)]
pub(super) struct Miss {
    /// `edit`, `stale (outside)`, `ambiguous` and so on; empty for a reason that is told as it is.
    pub class: String,
    /// The share of the quote's lines that were aligned with the file.
    pub percent: Option<u8>,
    /// First and last line (1-based, in the text before the patch) of the part of the file the hunk
    /// was aimed at.
    pub window: Option<(usize, usize)>,
    /// What is wrong, in one or two sentences.
    pub note: String,
    /// Lines printed below the note: the rows that differ, the window to quote again.
    pub rows: Vec<String>,
}

impl Miss {
    /// A miss told by its reason alone.
    pub(super) fn plain(note: String) -> Self {
        Self {
            class: String::new(),
            percent: None,
            window: None,
            note,
            rows: Vec::new(),
        }
    }

    fn of(class: &str, note: String) -> Self {
        Self {
            class: class.to_string(),
            ..Self::plain(note)
        }
    }
}

/// What a hunk was aimed at.
pub(super) struct Aim<'a> {
    pub path: &'a str,
    /// The text the hunk applies to.
    pub text: &'a str,
    pub old: &'a str,
    /// The hunk's `start_line`.
    pub hint: Option<usize>,
    /// The definition the hunk is limited to: its name and its first and last line (1-based).
    pub scope: Option<(&'a str, usize, usize)>,
    /// What the exact search said, kept for a quote that is outside its scope.
    pub reason: &'a str,
}

/// The miss of a quote that is nowhere in the text, even up to whitespace. `number` maps a 1-based
/// line of the text to the line it had before the patch.
pub(super) fn not_found(
    aim: &Aim<'_>,
    versions: &[Version],
    number: &dyn Fn(usize) -> usize,
) -> Miss {
    if aim.scope.is_some() && aim.text.contains(aim.old) {
        return Miss::of("scope", aim.reason.to_string());
    }
    let file: Vec<&str> = aim.text.split_inclusive('\n').map(align::body).collect();
    let quote: Vec<&str> = aim.old.split_inclusive('\n').map(align::body).collect();
    let (from, to) = match aim.scope {
        Some((_, first, last)) => (
            first.saturating_sub(1).min(file.len()),
            last.min(file.len()),
        ),
        None => (0, file.len()),
    };
    let window = &file[from..to.max(from)];
    let stale = rebase::stale_cause(aim.text, aim.old, aim.hint, versions);
    let Some(found) = align::diagnose(&quote, window) else {
        return no_window(aim, stale);
    };
    let at = |j: usize| number(from + j + 1);

    let swapped = found.swapped(&quote, window);
    let tags = found.tags(&quote, window);
    let mut diffs: Vec<(Option<usize>, String, String)> = Vec::new();
    for pair in &found.pairs {
        match *pair {
            Pair::Same { .. } => {}
            Pair::Edit { quote: i, file: j } => {
                diffs.push((Some(at(j)), window[j].to_string(), quote[i].to_string()));
            }
            Pair::Extra { quote: i } => {
                diffs.push((None, "(no such line)".to_string(), quote[i].to_string()));
            }
            Pair::Gap { file: j } => {
                diffs.push((
                    Some(at(j)),
                    window[j].to_string(),
                    "(not quoted)".to_string(),
                ));
            }
        }
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some((_, lines)) = &stale {
        parts.push(format!(
            "the file changed since you read it: lines {lines} differ from what you quoted"
        ));
    }
    for tag in &tags {
        match tag {
            Tag::Edit => {
                let edits = found
                    .pairs
                    .iter()
                    .filter(|pair| matches!(pair, Pair::Edit { .. }))
                    .count();
                parts.push(if edits == 1 {
                    "1 line differs".to_string()
                } else {
                    format!("{edits} lines differ")
                });
            }
            Tag::Gap => {
                if let Some((q0, q1, f0, f1, skipped)) = first_gap(&found.pairs) {
                    parts.push(format!(
                        "your lines {} and {} are {skipped} lines apart in the file (at {} and {}); quote the lines between them or use a `...` line",
                        q0 + 1,
                        q1 + 1,
                        at(f0),
                        at(f1)
                    ));
                }
            }
            Tag::Extra => {
                let lines: Vec<usize> = found
                    .pairs
                    .iter()
                    .filter_map(|pair| match *pair {
                        Pair::Extra { quote: i } if !swapped.iter().any(|&(q, _)| q == i) => {
                            Some(i + 1)
                        }
                        _ => None,
                    })
                    .collect();
                parts.push(format!(
                    "your line{} {} not in the file window",
                    if lines.len() == 1 { "" } else { "s" },
                    list(&lines, if lines.len() == 1 { "is" } else { "are" })
                ));
            }
            Tag::Order => {
                if let Some(&(q, f)) = swapped.first() {
                    parts.push(format!(
                        "your line {} is at line {} of the file, which is not where your quote puts it",
                        q + 1,
                        at(f)
                    ));
                }
            }
        }
    }
    if !found.rivals.is_empty() {
        let shown: Vec<String> = found
            .rivals
            .iter()
            .take(3)
            .map(|&(a, b)| format!("{}..{}", at(a), at(b)))
            .collect();
        parts.push(format!(
            "the window at lines {} fits as well",
            shown.join(", ")
        ));
    }
    let class = match &stale {
        Some((cause, _)) => format!("stale ({cause})"),
        None if tags.is_empty() => "spacing".to_string(),
        None => tags
            .iter()
            .map(|tag| tag.word())
            .collect::<Vec<_>>()
            .join("+"),
    };
    let note = if tags.is_empty() {
        let seen = format!(
            "its lines match lines {}..{} up to whitespace, which may not be changed inside a string or comment",
            at(found.first),
            at(found.last)
        );
        parts.insert(0, seen);
        format!("`old_text` not found byte for byte; {}", parts.join("; "))
    } else {
        format!("`old_text` not found; {}", parts.join("; "))
    };

    let width = diffs
        .iter()
        .filter_map(|diff| diff.0)
        .max()
        .map_or(1, |n| n.to_string().len());
    let pad = 5 + width;
    let mut rows: Vec<String> = Vec::new();
    for (n, filed, yours) in diffs.iter().take(SHOWN_ROWS) {
        let label = match n {
            Some(n) => format!("file {n:>width$}"),
            None => format!("file {:>width$}", "-"),
        };
        rows.push(format!("  {label} | {}", cut(filed)));
        rows.push(format!("  {:<pad$} | {}", "yours", cut(yours)));
    }
    if diffs.len() > SHOWN_ROWS {
        rows.push(format!(
            "  ... and {} more differing lines",
            diffs.len() - SHOWN_ROWS
        ));
    }
    if diffs.len() > 2 {
        rows.push(format!(
            "  the file at lines {}..{}, to quote again:",
            at(found.first),
            at(found.last)
        ));
        let stop = found.last.min(found.first + SHOWN_WINDOW - 1);
        for j in found.first..=stop {
            rows.push(format!("    {}\t{}", at(j), window[j]));
        }
        if stop < found.last {
            rows.push(format!("    ... {} more lines", found.last - stop));
        }
    }
    Miss {
        class,
        percent: Some(found.percent),
        window: Some((at(found.first), at(found.last))),
        note,
        rows,
    }
}

/// The miss of a quote that no window of the file resembles.
fn no_window(aim: &Aim<'_>, stale: Option<(String, String)>) -> Miss {
    let mut note = String::from("`old_text` not found; no close window in the file");
    if let Some((cause, lines)) = &stale {
        note.push_str(&format!(
            ", and the file changed since you read it ({cause}): lines {lines}"
        ));
    }
    if let Some((name, first, last)) = nearest(aim.path, aim.text, aim.old) {
        note.push_str(&format!(
            "; the nearest definition is `{name}` at lines {first}..{last}"
        ));
    }
    let class = match stale {
        Some((cause, _)) => format!("stale ({cause})"),
        None => "misquote".to_string(),
    };
    Miss::of(&class, note)
}

/// The definition whose name the quote mentions most, the smaller one on a tie: where a quote that
/// resembles nothing was probably aimed.
fn nearest(path: &str, text: &str, old: &str) -> Option<(String, usize, usize)> {
    let syntax = registry::for_path(path)?;
    let all = symbols::symbols(syntax, text)?;
    let words: Vec<&str> = old
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| word.len() >= 3)
        .collect();
    all.iter()
        .filter_map(|symbol| {
            let hits = words.iter().filter(|word| **word == symbol.name).count();
            (hits > 0).then_some((hits, symbol))
        })
        .max_by_key(|(hits, symbol)| (*hits, Reverse(symbol.last - symbol.first)))
        .map(|(_, symbol)| (qualified(symbol), symbol.first, symbol.last))
}

/// The name that `ReplaceSymbol` accepts for `symbol`: `Type::name` inside a type or module.
fn qualified(symbol: &Symbol) -> String {
    if symbol.scope.is_empty() {
        symbol.name.clone()
    } else {
        format!("{}::{}", symbol.scope, symbol.name)
    }
}

/// The first run of file lines that the quote skipped between two of its adjacent lines: the quote
/// lines on either side, the file lines they aligned with, and how many lines lie between.
fn first_gap(pairs: &[Pair]) -> Option<(usize, usize, usize, usize, usize)> {
    let mut before: Option<(usize, usize)> = None;
    let mut skipped = 0;
    for pair in pairs {
        match *pair {
            Pair::Same { quote, file } | Pair::Edit { quote, file } => {
                if skipped > 0
                    && let Some((q0, f0)) = before
                {
                    return Some((q0, quote, f0, file, skipped));
                }
                before = Some((quote, file));
            }
            Pair::Gap { .. } => skipped += 1,
            Pair::Extra { .. } => {}
        }
    }
    None
}

/// `5`, `5 and 7`, `5, 7 and 9`, then `verb`.
fn list(numbers: &[usize], verb: &str) -> String {
    format!("{} {verb}", join_numbers(numbers))
}

fn join_numbers(numbers: &[usize]) -> String {
    let words: Vec<String> = numbers.iter().map(ToString::to_string).collect();
    match words.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `line` cut to [`CUT`] characters, with the end of the cut shown by `...`.
fn cut(line: &str) -> String {
    let line = line.trim_end();
    if line.chars().count() <= CUT {
        return line.to_string();
    }
    let head: String = line.chars().take(CUT).collect();
    format!("{head}...")
}

/// The miss of a quote that fits several places. `starts` are the byte offsets of the places in
/// `text` and `old` is the quote; each place is told with the fewest lines of context above and
/// below that tell it from the others.
pub(super) fn ambiguous(
    text: &str,
    old: &str,
    starts: &[usize],
    reason: String,
    number: &dyn Fn(usize) -> usize,
) -> Miss {
    let file: Vec<&str> = text.split_inclusive('\n').map(align::body).collect();
    let lines: Vec<usize> = starts
        .iter()
        .map(|&at| {
            text.as_bytes()[..at]
                .iter()
                .filter(|&&b| b == b'\n')
                .count()
        })
        .collect();
    let len = old.split_inclusive('\n').count();
    let found = align::extensions(&file, len, &lines);
    let mut rows = Vec::new();
    for (line, extension) in lines.iter().zip(&found).take(5) {
        let at = number(line + 1);
        rows.push(match extension {
            Some(extension) => format!("  at line {at}: add {}", describe(*extension)),
            None => format!("  at line {at}: no short context tells it apart; set `start_line`"),
        });
    }
    Miss {
        rows,
        ..Miss::of("ambiguous", reason)
    }
}

/// `1 line above`, `2 lines below`, `1 line above and 2 lines below`.
fn describe(extension: align::Extension) -> String {
    let noun = |n: usize| if n == 1 { "line" } else { "lines" };
    match (extension.above, extension.below) {
        (above, 0) => format!("{above} {} above", noun(above)),
        (0, below) => format!("{below} {} below", noun(below)),
        (above, below) => format!(
            "{above} {} above and {below} {} below",
            noun(above),
            noun(below)
        ),
    }
}

/// The miss of a quote that holds lines an earlier hunk of the same call rewrote. `original` is the
/// text before the call and `earlier` the `(hunk, first line, last line)` of every hunk placed so
/// far, in the lines of that text. `None` unless the quote is in `original` at exactly one place
/// that touches an earlier hunk (when it is there more than once, the one at `hint`).
pub(super) fn overlap(
    original: &str,
    old: &str,
    hint: Option<usize>,
    earlier: &[(usize, usize, usize)],
) -> Option<Miss> {
    let len = old.split_inclusive('\n').count().max(1);
    let mut places = 0;
    let mut hits: Vec<(usize, usize, usize)> = Vec::new();
    for (at, _) in original.match_indices(old) {
        places += 1;
        let first = original.as_bytes()[..at]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
            + 1;
        let last = first + len - 1;
        if let Some(&(hunk, _, _)) = earlier
            .iter()
            .find(|&&(_, from, to)| from <= last && first <= to)
        {
            hits.push((first, last, hunk));
        }
    }
    let [(first, last, hunk)] = hits.as_slice() else {
        return None;
    };
    if places > 1 && hint != Some(*first) {
        return None;
    }
    Some(Miss {
        window: Some((*first, *last)),
        ..Miss::of(
            "overlap",
            format!(
                "`old_text` not found; it quotes lines {first}..{last} that hunk {hunk} of this call rewrote; merge the two hunks, or quote those lines as hunk {hunk} leaves them"
            ),
        )
    })
}

/// The miss of a quote whose placeholder lines could not be expanded; `why` says what is missing.
pub(super) fn elision(why: String) -> Miss {
    Miss::of("elision", format!("`old_text` not found; {why}"))
}

/// The reason of the error for a patch of `total` hunks of which `failures` (0-based index and miss)
/// did not place. `placed` holds every other hunk with the lines it covers, in the lines before the
/// patch: those that touch the window of a failed hunk are unverified, because the failed hunk
/// would have been checked against text that its own repair may change.
pub(super) fn report(
    total: usize,
    failures: &[(usize, Miss)],
    placed: &[(usize, Option<(usize, usize)>)],
) -> String {
    let failed: Vec<usize> = failures.iter().map(|(index, _)| index + 1).collect();
    let mut verified: Vec<usize> = Vec::new();
    let mut unverified: Vec<(usize, (usize, usize), usize)> = Vec::new();
    for &(number, span) in placed {
        let touched = span.and_then(|(first, last)| {
            failures.iter().find_map(|(index, miss)| match miss.window {
                Some((from, to)) if from <= last && first <= to => Some(index + 1),
                _ => None,
            })
        });
        match (touched, span) {
            (Some(hunk), Some(lines)) => unverified.push((number, lines, hunk)),
            _ => verified.push(number),
        }
    }

    let mut head = if total == 1 {
        "hunk 1 failed".to_string()
    } else {
        let noun = if failed.len() == 1 { "hunk" } else { "hunks" };
        format!("{noun} {} of {total} failed", join_numbers(&failed))
    };
    if !verified.is_empty() {
        head.push_str(&format!("; {} verified", join_numbers(&verified)));
    }
    if !unverified.is_empty() {
        let numbers: Vec<usize> = unverified.iter().map(|entry| entry.0).collect();
        head.push_str(&format!("; {} unverified", join_numbers(&numbers)));
    }
    head.push_str("; nothing was written");

    let mut lines = vec![head];
    for (index, miss) in failures {
        let number = index + 1;
        if miss.class.is_empty() {
            lines.push(format!("hunk {number}: {}", miss.note));
        } else {
            let mut title = format!("hunk {number}  {}", miss.class);
            if let Some(percent) = miss.percent {
                title.push_str(&format!("  {percent}%"));
            }
            match miss.window {
                Some((from, to)) if from == to => title.push_str(&format!("  line {from}")),
                Some((from, to)) => title.push_str(&format!("  lines {from}..{to}")),
                None => {}
            }
            lines.push(title);
            lines.push(format!("  {}", miss.note));
        }
        lines.extend(miss.rows.iter().cloned());
    }
    for (number, (first, last), hunk) in &unverified {
        lines.push(format!(
            "hunk {number}  unverified  lines {first}..{last} touch the window of failed hunk {hunk}"
        ));
    }
    lines.join("\n")
}
