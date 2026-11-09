//! Placing a hunk whose `old_text` leaves lines out with a placeholder line.
//!
//! An agent that quotes a long block often writes `// ... existing code ...` for the middle of it.
//! A line that is only an optional comment marker, `...` or `…`, and a few words, and that the file
//! does not hold itself, is such a gap. The lines before the first gap (the head) and after the last
//! one (the tail) are matched in the file, each at one place only, the tail after the head and within
//! [`SPAN`] lines of it; blocks between two gaps likewise. Each gap of `old_text` must be paired, in
//! order, with a placeholder line of `new_text`, and that line is replaced by the file lines the gap
//! stood for, so what the agent never named is neither deleted nor rewritten. A placeholder that only
//! `old_text` has would delete lines nobody quoted, so that hunk is diagnosed and not placed.

use std::collections::HashSet;
use std::ops::Range;

use crate::common::align;

use super::repair::Window;

/// Most lines that the placed span may cover, from the first line of the head to the last of the
/// tail.
const SPAN: usize = 120;
/// Most words that a placeholder line may have besides its marker and dots.
const WORDS: usize = 8;

/// Whether `line` is only a placeholder for lines left out: an optional comment marker, `...` or
/// `…`, and optional words (`// ... existing code ...`, `# ...`, `...`). Without a comment marker the
/// dots stand alone or are followed by a blank, so that a spread such as `...rest,` is code.
pub(crate) fn is_placeholder(line: &str) -> bool {
    let mut rest = line.trim();
    let mut marked = false;
    for marker in ["<!--", "/*", "//", "--", "#", ";", "%"] {
        if let Some(after) = rest.strip_prefix(marker) {
            rest = after.trim_start();
            marked = true;
            break;
        }
    }
    let rest = rest
        .trim_end_matches("*/")
        .trim_end_matches("-->")
        .trim_end();
    let Some(words) = rest.strip_prefix("...").or_else(|| rest.strip_prefix('…')) else {
        return false;
    };
    if !marked && !words.is_empty() && !words.starts_with(char::is_whitespace) {
        return false;
    }
    words.split_whitespace().count() <= WORDS
        && words
            .chars()
            .all(|c| c.is_alphanumeric() || c.is_whitespace() || "-_'.,:()".contains(c))
}

/// The placeholder lines of `new` that `text` does not hold: written to the file as they stand.
pub(crate) fn stand_ins_absent(text: &str, new: &str) -> Vec<String> {
    let present: HashSet<&str> = text.lines().map(str::trim).collect();
    new.lines()
        .map(str::trim)
        .filter(|line| is_placeholder(line) && !present.contains(*line))
        .map(str::to_string)
        .collect()
}

/// Whether the lines of `block` are the lines of `bodies` from index `at`, up to trailing blanks.
fn block_at(block: &[&str], bodies: &[&str], at: usize) -> bool {
    at + block.len() <= bodies.len()
        && block
            .iter()
            .zip(&bodies[at..])
            .all(|(quoted, filed)| quoted.trim_end() == filed.trim_end())
}

/// Expand the placeholder lines of the quote `old` over `text`. `None` when the quote has none.
/// `Some(Err(why))` when it has and they cannot be expanded; the reason is for the report. A place
/// outside `scope` (the byte range of the hunk's `symbol`) is never taken.
pub(super) fn expand(
    text: &str,
    (old, new): (&str, &str),
    scope: Option<&Range<usize>>,
) -> Option<Result<Window, String>> {
    let file: Vec<&str> = text.split_inclusive('\n').collect();
    let bodies: Vec<&str> = file.iter().map(|line| align::body(line)).collect();
    let present = |line: &str| bodies.iter().any(|b| b.trim() == line.trim());
    let stand_ins = |lines: &[&str]| -> Vec<usize> {
        (0..lines.len())
            .filter(|&i| is_placeholder(lines[i]) && !present(lines[i]))
            .collect()
    };
    let quote: Vec<&str> = old.split_inclusive('\n').collect();
    let qb: Vec<&str> = quote.iter().map(|line| align::body(line)).collect();
    let gaps = stand_ins(qb.as_slice());
    if gaps.is_empty() {
        return None;
    }
    let after: Vec<&str> = new.split_inclusive('\n').collect();
    let ab: Vec<&str> = after.iter().map(|line| align::body(line)).collect();
    let marks = stand_ins(ab.as_slice());

    let mut starts: Vec<usize> = Vec::with_capacity(file.len());
    let mut offset = 0;
    for line in &file {
        starts.push(offset);
        offset += line.len();
    }
    let in_scope =
        |line: usize| scope.is_none_or(|s| starts[line] >= s.start && starts[line] < s.end);

    let seated = || -> Result<Window, String> {
        if marks.len() < gaps.len() {
            return Err(format!(
                "old_text has {} placeholder line(s) and new_text keeps {}; a placeholder left out of new_text would delete the lines it stands for, so keep one in new_text for each",
                gaps.len(),
                marks.len()
            ));
        }
        if marks.len() > gaps.len() {
            return Err(format!(
                "new_text has {} placeholder lines and old_text {}; each placeholder of new_text must stand for one of old_text",
                marks.len(),
                gaps.len()
            ));
        }
        let mut blocks: Vec<Range<usize>> = Vec::new();
        let mut from = 0;
        for &gap in &gaps {
            blocks.push(from..gap);
            from = gap + 1;
        }
        blocks.push(from..qb.len());
        if blocks[0].is_empty() {
            return Err("its first line is a placeholder; quote the lines before it".to_string());
        }
        if blocks[blocks.len() - 1].is_empty() {
            return Err("its last line is a placeholder; quote the lines after it".to_string());
        }
        if blocks.iter().any(|block| block.is_empty()) {
            return Err("two placeholder lines are next to each other".to_string());
        }

        let head = &qb[blocks[0].clone()];
        let heads: Vec<usize> = (0..bodies.len())
            .filter(|&at| block_at(head, &bodies, at) && in_scope(at))
            .collect();
        let [first] = heads.as_slice() else {
            return Err(if heads.is_empty() {
                "the lines before the first placeholder are not in the file as quoted".to_string()
            } else {
                format!(
                    "the lines before the first placeholder match {} places; quote more of them",
                    heads.len()
                )
            });
        };
        let first = *first;
        let bound = (first + SPAN).min(bodies.len());
        let mut cursor = first + head.len();
        let mut hidden: Vec<(usize, usize)> = Vec::new();
        for (k, block) in blocks.iter().enumerate().skip(1) {
            let lines = &qb[block.clone()];
            let found: Vec<usize> = (cursor..bound)
                .filter(|&at| at + lines.len() <= bound && block_at(lines, &bodies, at))
                .collect();
            let [at] = found.as_slice() else {
                return Err(if found.is_empty() {
                    format!(
                        "the lines after placeholder {k} do not follow the lines before it within {SPAN} lines"
                    )
                } else {
                    format!(
                        "the lines after placeholder {k} match {} places; quote more of them",
                        found.len()
                    )
                });
            };
            hidden.push((cursor, *at));
            cursor = *at + lines.len();
        }

        let mut replacement = String::new();
        let mut next = 0;
        for (i, line) in after.iter().enumerate() {
            if marks.contains(&i) {
                let (from, to) = hidden[next];
                next += 1;
                for kept in &file[from..to] {
                    replacement.push_str(kept);
                }
            } else {
                replacement.push_str(line);
            }
        }
        let last = cursor - 1;
        let at = starts[first];
        let end = if old.ends_with('\n') {
            starts[last] + file[last].len()
        } else {
            starts[last] + bodies[last].len()
        };
        if scope.is_some_and(|s| end > s.end) {
            return Err(
                "the lines it matches reach outside the `symbol` it is limited to".to_string(),
            );
        }
        let stood: usize = hidden.iter().map(|&(from, to)| to - from).sum();
        Ok(Window {
            at,
            old: text[at..end].to_string(),
            new: replacement,
            how: format!(
                "elision: {} stood for {stood} lines",
                if gaps.len() == 1 {
                    "placeholder"
                } else {
                    "placeholders"
                }
            ),
        })
    };
    Some(seated())
}
