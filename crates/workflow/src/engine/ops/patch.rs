//! Applying [`PatchHunk`]s to text.
//!
//! Each hunk is applied to the result of the previous one. `old_text` must match exactly
//! once; if it matches several places, `start_line` (1-based, in the pre-patch text) selects
//! the nearest one. A `symbol` restricts the search to one definition of the file. A hunk whose `old_text` uses `\n` is retried with `\r\n` when the
//! file uses CRLF.
//!
//! [`apply_hunks`] is exact and is what replays the ledger. [`apply_hunks_tolerant`] is for hunks
//! an agent wrote: when `old_text` is not found byte for byte it also accepts a quote whose lines
//! differ only in whitespace and a hunk whose `new_text` is already in the file (see `repair`), and
//! it checks every hunk before it reports, so one reply names all the hunks that need a fix.

use crate::common::{AgentError, LineNumber, Result};

use super::PatchHunk;
use super::rebase;
pub use super::rebase::{History, Version};
use super::repair;
use super::symbol_edit::definition;

/// A previously applied hunk in the current buffer: its end byte offset and the net newline
/// change (`new_newlines - old_newlines`) it introduced.
struct AppliedEdit {
    end_byte: usize,
    line_delta: i64,
}

/// Apply `hunks` to `text`. Returns the new text and the hunks as applied, with `lines`
/// recorded as the 1-based line range in the pre-patch `text`.
pub fn apply_hunks(
    path: &str,
    text: &str,
    hunks: &[PatchHunk],
) -> Result<(String, Vec<PatchHunk>)> {
    let done = apply_all(path, text, hunks, None)?;
    Ok((done.text, done.hunks))
}

/// What [`apply_hunks_tolerant`] made of a patch.
#[derive(Debug)]
pub struct Applied {
    pub text: String,
    /// The hunks as they were applied, in the pre-patch coordinates of [`apply_hunks`].
    pub hunks: Vec<PatchHunk>,
    /// The hunks that were not placed byte for byte.
    pub repairs: Vec<Repair>,
}

/// A hunk that was placed by a rule other than the exact match.
#[derive(Debug)]
pub struct Repair {
    /// 1-based position of the hunk in the patch.
    pub hunk: usize,
    /// How it was placed: `spacing: indent +4 spaces`, `already applied at 41`.
    pub how: String,
}

/// How one hunk was placed.
enum Placed {
    Exact(PatchHunk),
    Repaired(PatchHunk, String),
    /// The text already holds the replacement; nothing changed.
    Already(String),
}

/// Apply `hunks` like [`apply_hunks`], for hunks an agent wrote. A hunk whose `old_text` is not
/// in the text byte for byte is still placed when it is provably the same text (see `repair`),
/// and every hunk is checked before an error is returned, so the error names all that failed.
/// A patch with any failed hunk changes nothing.
pub fn apply_hunks_tolerant(
    path: &str,
    text: &str,
    hunks: &[PatchHunk],
    history: &History<'_>,
) -> Result<Applied> {
    apply_all(path, text, hunks, Some(history))
}

fn apply_all(
    path: &str,
    text: &str,
    hunks: &[PatchHunk],
    history: Option<&History<'_>>,
) -> Result<Applied> {
    let mut current = text.to_string();
    let mut applied = Vec::with_capacity(hunks.len());
    let mut repairs = Vec::new();
    let mut failures: Vec<(usize, String)> = Vec::new();
    let mut edits: Vec<AppliedEdit> = Vec::new();

    for (index, hunk) in hunks.iter().enumerate() {
        match apply_one(path, &mut current, &mut edits, hunk, history) {
            Ok(Placed::Exact(done)) => applied.push(done),
            Ok(Placed::Repaired(done, how)) => {
                applied.push(done);
                repairs.push(Repair {
                    hunk: index + 1,
                    how,
                });
            }
            Ok(Placed::Already(how)) => repairs.push(Repair {
                hunk: index + 1,
                how,
            }),
            Err(reason) if history.is_some() => failures.push((index, reason)),
            Err(reason) => {
                return Err(AgentError::PatchConflict {
                    path: path.to_string(),
                    reason: format!("hunk {}: {reason}", index + 1),
                });
            }
        }
    }
    if !failures.is_empty() {
        return Err(failure_report(path, hunks.len(), &failures));
    }
    Ok(Applied {
        text: current,
        hunks: applied,
        repairs,
    })
}

/// One error for every hunk that did not place. The hunks after a failed one were checked against
/// the text without it, and nothing was written.
fn failure_report(path: &str, total: usize, failures: &[(usize, String)]) -> AgentError {
    let reason = match failures {
        [(index, reason)] if total == 1 => format!("hunk {}: {reason}", index + 1),
        [(index, reason)] => format!(
            "hunk {}: {reason} (the other {} hunks matched; nothing was written)",
            index + 1,
            total - 1
        ),
        _ => {
            let lines: Vec<String> = failures
                .iter()
                .map(|(index, reason)| format!("  hunk {}: {reason}", index + 1))
                .collect();
            format!(
                "{} of {total} hunks did not match and nothing was written; a hunk after a failed one was checked without it:\n{}",
                failures.len(),
                lines.join("\n")
            )
        }
    };
    AgentError::PatchConflict {
        path: path.to_string(),
        reason,
    }
}

/// Place one hunk in `current`, or say why it cannot be placed.
fn apply_one(
    path: &str,
    current: &mut String,
    edits: &mut Vec<AppliedEdit>,
    hunk: &PatchHunk,
    history: Option<&History<'_>>,
) -> std::result::Result<Placed, String> {
    if hunk.old_text.is_empty() {
        if !current.is_empty() {
            return Err("`old_text` is empty (only valid when patching an empty file)".into());
        }
        let new = hunk.new_text.clone();
        let new_nl = count_newlines(&new);
        *current = new.clone();
        record_edit(edits, 0, 0, current.len(), new_nl);
        return Ok(Placed::Exact(PatchHunk {
            start_line: Some(LineNumber::ONE),
            old_text: String::new(),
            new_text: new,
            symbol: None,
        }));
    }
    let (old, new) = adapt_line_endings(current.as_str(), &hunk.old_text, &hunk.new_text);
    let scope = match hunk
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|symbol| !symbol.is_empty())
    {
        Some(symbol) => Some(symbol_scope(path, current.as_str(), symbol)?),
        None => None,
    };
    match locate(
        current.as_str(),
        &old,
        hunk.start_line,
        edits.as_slice(),
        scope.as_ref(),
    ) {
        Ok(at) => Ok(Placed::Exact(place(current, edits, at, old, new))),
        Err(miss) => {
            let nowhere = exact_positions(current.as_str(), &old, scope.as_ref()).is_empty();
            match history {
                Some(history) if nowhere => {
                    let quote = (old.as_str(), new.as_str());
                    repair_hunk(
                        path,
                        current,
                        edits,
                        (hunk.start_line, scope.as_ref()),
                        quote,
                        (history, miss),
                    )
                }
                _ => Err(miss),
            }
        }
    }
}

/// Replace `old` at byte offset `at` of `current` with `new` and record the edit.
fn place(
    current: &mut String,
    edits: &mut Vec<AppliedEdit>,
    at: usize,
    old: String,
    new: String,
) -> PatchHunk {
    let start_line = pre_patch_line(current.as_str(), at, edits.as_slice());
    let line_delta = count_newlines(&new) - count_newlines(&old);
    current.replace_range(at..at + old.len(), &new);
    record_edit(edits, at, old.len(), new.len(), line_delta);
    PatchHunk {
        start_line: Some(start_line),
        old_text: old,
        new_text: new,
        symbol: None,
    }
}

/// A hunk whose `old_text` is nowhere in the text byte for byte: place it where its lines are the
/// text's lines up to whitespace, note that the text already holds its replacement, or seat the
/// lines it changes (see `rebase`). `miss` is the reason to report when none holds.
fn repair_hunk(
    path: &str,
    current: &mut String,
    edits: &mut Vec<AppliedEdit>,
    (hint, scope): (Option<LineNumber>, Option<&Scope<'_>>),
    (old, new): (&str, &str),
    (history, miss): (&History<'_>, String),
) -> std::result::Result<Placed, String> {
    let range = scope.map(|s| s.span.clone());
    let spans = repair::text_spans(path, current.as_str());
    let windows = {
        let spacing = repair::spacing_windows(current.as_str(), old, new, range.as_ref(), &spans);
        if spacing.is_empty() {
            repair::squeezed_windows(path, current.as_str(), (old, new), range.as_ref(), &spans)
        } else {
            spacing
        }
    };
    if !windows.is_empty() {
        let starts: Vec<usize> = windows.iter().map(|w| w.at).collect();
        let at = pick(
            current.as_str(),
            &starts,
            hint,
            edits.as_slice(),
            scope.is_some(),
        )?;
        let Some(window) = windows.into_iter().find(|w| w.at == at) else {
            return Err(miss);
        };
        let how = window.how;
        let done = place(current, edits, window.at, window.old, window.new);
        return Ok(Placed::Repaired(done, how));
    }
    if let Some(at) = repair::already_applied(current.as_str(), new, range.as_ref(), &spans) {
        let line = pre_patch_line(current.as_str(), at, edits.as_slice());
        return Ok(Placed::Already(format!("already applied at {line}")));
    }
    let versions = history.versions();
    let hint_line = hint.map(|line| line.get() as usize);
    if let Some(window) = rebase::rebase(
        current.as_str(),
        (old, new),
        (range.as_ref(), hint_line),
        versions,
    ) {
        let how = window.how;
        let done = place(current, edits, window.at, window.old, window.new);
        return Ok(Placed::Repaired(done, how));
    }
    Err(miss)
}

/// Byte offsets where `old` occurs in `text`, inside the symbol's lines when there is a `scope`.
fn exact_positions(text: &str, old: &str, scope: Option<&Scope<'_>>) -> Vec<usize> {
    text.match_indices(old)
        .map(|(i, _)| i)
        .filter(|&i| scope.is_none_or(|s| i >= s.span.start && i + old.len() <= s.span.end))
        .collect()
}

/// Apply the inverse of `hunks` (`new_text -> old_text`) to `after_text` to reconstruct the
/// pre-patch text.
///
/// Hunks are inverted in forward order (`0..hunks.len()`). Because hunks `0..k` have already
/// been reverted to their pre-patch `old_text` when hunk `k` runs, the start of `hunk.lines` directly
/// matches the 1-based line number in the working buffer.
pub fn apply_hunks_reverse(path: &str, after_text: &str, hunks: &[PatchHunk]) -> Result<String> {
    let mut current = after_text.to_string();
    let no_edits: [AppliedEdit; 0] = [];

    for (index, hunk) in hunks.iter().enumerate() {
        let conflict = |reason: String| AgentError::PatchConflict {
            path: path.to_string(),
            reason: format!("reverse hunk {}: {reason}", index + 1),
        };
        if hunk.new_text.is_empty() {
            if current.is_empty() {
                current = hunk.old_text.clone();
            } else {
                let at = byte_offset_of_line(&current, hunk.start_line.unwrap_or(LineNumber::ONE));
                current.insert_str(at, &hunk.old_text);
            }
            continue;
        }
        let (new_in_file, old_to_restore) =
            adapt_line_endings(&current, &hunk.new_text, &hunk.old_text);
        let at =
            locate(&current, &new_in_file, hunk.start_line, &no_edits, None).map_err(conflict)?;
        current.replace_range(at..at + new_in_file.len(), &old_to_restore);
    }
    Ok(current)
}

fn adapt_line_endings(text: &str, find: &str, replace: &str) -> (String, String) {
    let wants_crlf = text.contains("\r\n") && find.contains('\n') && !find.contains("\r\n");
    if wants_crlf {
        let crlf = |s: &str| s.replace("\r\n", "\n").replace('\n', "\r\n");
        (crlf(find), crlf(replace))
    } else {
        (find.to_string(), replace.to_string())
    }
}

fn count_newlines(s: &str) -> i64 {
    s.bytes().filter(|&b| b == b'\n').count() as i64
}

fn record_edit(
    edits: &mut Vec<AppliedEdit>,
    at: usize,
    old_len: usize,
    new_len: usize,
    line_delta: i64,
) {
    let byte_delta = new_len as isize - old_len as isize;
    for edit in edits.iter_mut() {
        if edit.end_byte > at {
            edit.end_byte = (edit.end_byte as isize + byte_delta).max(0) as usize;
        }
    }
    edits.push(AppliedEdit {
        end_byte: at + new_len,
        line_delta,
    });
}

/// 1-based line number of byte offset `at` in the original pre-patch text.
fn pre_patch_line(current: &str, at: usize, edits: &[AppliedEdit]) -> LineNumber {
    let cur_line = line_number(current, at).get() as i64;
    let delta: i64 = edits
        .iter()
        .filter(|e| e.end_byte <= at)
        .map(|e| e.line_delta)
        .sum();
    let num = (cur_line - delta).max(1) as u64;
    LineNumber::new(num).unwrap_or(LineNumber::ONE)
}

/// Byte offset of the start of 1-based `target_line` (clamped to `text.len()`).
pub(super) fn byte_offset_of_line(text: &str, target_line: LineNumber) -> usize {
    if target_line.get() <= 1 {
        return 0;
    }
    let needed = target_line.to_zero_based();
    let mut seen = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            seen += 1;
            if seen == needed {
                return i + 1;
            }
        }
    }
    text.len()
}

/// The part of a file that a hunk is restricted to by its `symbol`.
struct Scope<'a> {
    symbol: &'a str,
    /// Byte range of the definition's lines.
    span: std::ops::Range<usize>,
    first: usize,
    last: usize,
}

/// The lines of the one definition `symbol` names in `text`, the buffer the hunk applies to.
fn symbol_scope<'a>(
    path: &str,
    text: &str,
    symbol: &'a str,
) -> std::result::Result<Scope<'a>, String> {
    let hit = definition(path, text, symbol)?;
    Ok(Scope {
        symbol,
        span: byte_offset_of_line(
            text,
            LineNumber::new(hit.first as u64).unwrap_or(LineNumber::ONE),
        )
            ..byte_offset_of_line(
                text,
                LineNumber::new(hit.last as u64 + 1).unwrap_or(LineNumber::ONE),
            ),
        first: hit.first,
        last: hit.last,
    })
}

/// Byte offset of the single intended match, or a reason string.
fn locate(
    text: &str,
    old: &str,
    hint: Option<LineNumber>,
    edits: &[AppliedEdit],
    scope: Option<&Scope<'_>>,
) -> std::result::Result<usize, String> {
    let positions = exact_positions(text, old, scope);
    if positions.is_empty() {
        return Err(match scope {
            Some(s) => format!(
                "`old_text` not found inside `{}` (lines {}..{}){}",
                s.symbol,
                s.first,
                s.last,
                if text.contains(old) {
                    "; the text appears outside it"
                } else {
                    ""
                }
            ),
            None => not_found_reason(text, old),
        });
    }
    pick(text, &positions, hint, edits, scope.is_some())
}

/// The one of `positions` that a hunk means: the only one, or the one nearest to `hint`.
fn pick(
    text: &str,
    positions: &[usize],
    hint: Option<LineNumber>,
    edits: &[AppliedEdit],
    scoped: bool,
) -> std::result::Result<usize, String> {
    if let [only] = positions {
        return Ok(*only);
    }
    let n = positions.len();
    let lines: Vec<LineNumber> = positions
        .iter()
        .map(|&p| pre_patch_line(text, p, edits))
        .collect();
    if let Some(hint) = hint {
        let mut ranked: Vec<(u64, usize)> = lines
            .iter()
            .zip(positions)
            .map(|(&line, &pos)| (line.get().abs_diff(hint.get()), pos))
            .collect();
        ranked.sort_unstable();
        if ranked[0].0 < ranked[1].0 {
            return Ok(ranked[0].1);
        }
    }
    let shown: Vec<String> = lines.iter().take(5).map(ToString::to_string).collect();
    let advice = if scoped {
        "add surrounding context or set `start_line`"
    } else {
        "add surrounding context, set `start_line`, or set `symbol`"
    };
    Err(format!(
        "`old_text` matches {n} places (lines {}); {advice}",
        shown.join(", ")
    ))
}

fn not_found_reason(text: &str, old: &str) -> String {
    if let Some(first) = old.lines().map(str::trim).find(|l| !l.is_empty()) {
        let hits: Vec<String> = text
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(first))
            .map(|(i, _)| (i + 1).to_string())
            .take(5)
            .collect();
        if !hits.is_empty() {
            return format!(
                "`old_text` not found; its first line appears at line(s) {} but the full block differs (check whitespace and line endings)",
                hits.join(", ")
            );
        }
    }
    "`old_text` not found".into()
}

/// 1-based line number of byte offset `at`.
fn line_number(text: &str, at: usize) -> LineNumber {
    let count = text.as_bytes()[..at]
        .iter()
        .filter(|&&b| b == b'\n')
        .count();
    LineNumber::from_zero_based(count)
}
