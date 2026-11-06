//! Applying [`PatchHunk`]s to text.
//!
//! Each hunk is applied to the result of the previous one. `old_text` must match exactly
//! once; if it matches several places, `start_line` (1-based, in the pre-patch text) selects
//! the nearest one. A `symbol` restricts the search to one definition of the file. A hunk whose `old_text` uses `\n` is retried with `\r\n` when the
//! file uses CRLF.

use crate::common::{AgentError, Result};

use super::PatchHunk;
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
    let mut current = text.to_string();
    let mut applied = Vec::with_capacity(hunks.len());
    let mut edits: Vec<AppliedEdit> = Vec::new();

    for (index, hunk) in hunks.iter().enumerate() {
        let conflict = |reason: String| AgentError::PatchConflict {
            path: path.to_string(),
            reason: format!("hunk {}: {reason}", index + 1),
        };
        if hunk.old_text.is_empty() {
            if !current.is_empty() {
                return Err(conflict(
                    "`old_text` is empty (only valid when patching an empty file)".into(),
                ));
            }
            let new = hunk.new_text.clone();
            let new_nl = count_newlines(&new);
            current = new.clone();
            record_edit(&mut edits, 0, 0, current.len(), new_nl);
            applied.push(PatchHunk {
                start_line: Some(1),
                old_text: String::new(),
                new_text: new,
                symbol: None,
            });
            continue;
        }
        let (old, new) = adapt_line_endings(&current, &hunk.old_text, &hunk.new_text);
        let scope = match hunk
            .symbol
            .as_deref()
            .map(str::trim)
            .filter(|symbol| !symbol.is_empty())
        {
            Some(symbol) => Some(symbol_scope(path, &current, symbol).map_err(conflict)?),
            None => None,
        };
        let at =
            locate(&current, &old, hunk.start_line, &edits, scope.as_ref()).map_err(conflict)?;
        let start_line = pre_patch_line(&current, at, &edits);
        let line_delta = count_newlines(&new) - count_newlines(&old);
        current.replace_range(at..at + old.len(), &new);
        record_edit(&mut edits, at, old.len(), new.len(), line_delta);
        applied.push(PatchHunk {
            start_line: Some(start_line),
            old_text: old,
            new_text: new,
            symbol: None,
        });
    }
    Ok((current, applied))
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
                let at = byte_offset_of_line(&current, hunk.start_line.unwrap_or(1));
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
fn pre_patch_line(current: &str, at: usize, edits: &[AppliedEdit]) -> u64 {
    let cur_line = line_number(current, at) as i64;
    let delta: i64 = edits
        .iter()
        .filter(|e| e.end_byte <= at)
        .map(|e| e.line_delta)
        .sum();
    (cur_line - delta).max(1) as u64
}

/// Byte offset of the start of 1-based `target_line` (clamped to `text.len()`).
pub(super) fn byte_offset_of_line(text: &str, target_line: u64) -> usize {
    if target_line <= 1 {
        return 0;
    }
    let needed = (target_line - 1) as usize;
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
        span: byte_offset_of_line(text, hit.first as u64)
            ..byte_offset_of_line(text, hit.last as u64 + 1),
        first: hit.first,
        last: hit.last,
    })
}

/// Byte offset of the single intended match, or a reason string.
fn locate(
    text: &str,
    old: &str,
    hint: Option<u64>,
    edits: &[AppliedEdit],
    scope: Option<&Scope<'_>>,
) -> std::result::Result<usize, String> {
    let positions: Vec<usize> = text
        .match_indices(old)
        .map(|(i, _)| i)
        .filter(|&i| scope.is_none_or(|s| i >= s.span.start && i + old.len() <= s.span.end))
        .collect();
    match positions.len() {
        0 => Err(match scope {
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
        }),
        1 => Ok(positions[0]),
        n => {
            let lines: Vec<u64> = positions
                .iter()
                .map(|&p| pre_patch_line(text, p, edits))
                .collect();
            if let Some(hint) = hint {
                let mut ranked: Vec<(u64, usize)> = lines
                    .iter()
                    .zip(&positions)
                    .map(|(&line, &pos)| (line.abs_diff(hint), pos))
                    .collect();
                ranked.sort_unstable();
                if ranked[0].0 < ranked[1].0 {
                    return Ok(ranked[0].1);
                }
            }
            let shown: Vec<String> = lines.iter().take(5).map(u64::to_string).collect();
            let advice = if scope.is_some() {
                "add surrounding context or set `start_line`"
            } else {
                "add surrounding context, set `start_line`, or set `symbol`"
            };
            Err(format!(
                "`old_text` matches {n} places (lines {}); {advice}",
                shown.join(", ")
            ))
        }
    }
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
fn line_number(text: &str, at: usize) -> u64 {
    1 + text.as_bytes()[..at]
        .iter()
        .filter(|&&b| b == b'\n')
        .count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(old: &str, new: &str, line: Option<u64>) -> PatchHunk {
        PatchHunk {
            start_line: line,
            old_text: old.into(),
            new_text: new.into(),
            symbol: None,
        }
    }

    const RUST_TWICE: &str = "struct Store;\nimpl Store {\n    fn put(&self) {\n        log();\n    }\n}\nimpl Queue {\n    fn put(&self) {\n        log();\n    }\n}\n";

    #[test]
    fn symbol_restricts_the_search_to_one_definition() {
        assert!(apply_hunks("a.rs", RUST_TWICE, &[hunk("log();", "trace();", None)]).is_err());
        let scoped = PatchHunk {
            symbol: Some("Queue::put".into()),
            ..hunk("log();", "trace();", None)
        };
        let (out, applied) = apply_hunks("a.rs", RUST_TWICE, &[scoped]).unwrap();
        assert!(
            out.contains("impl Store {\n    fn put(&self) {\n        log();"),
            "{out}"
        );
        assert!(out.ends_with("        trace();\n    }\n}\n"), "{out}");
        assert_eq!(applied[0].start_line, Some(9));
        assert_eq!(applied[0].symbol, None);
        assert_eq!(
            apply_hunks_reverse("a.rs", &out, &applied).unwrap(),
            RUST_TWICE
        );
    }

    #[test]
    fn symbol_errors_say_what_went_wrong() {
        let with = |path: &str, symbol: &str, old: &str| {
            let hunk = PatchHunk {
                symbol: Some(symbol.into()),
                ..hunk(old, "x", None)
            };
            apply_hunks(path, RUST_TWICE, &[hunk])
                .unwrap_err()
                .to_string()
        };
        assert!(with("a.rs", "put", "log();").contains("names 2 definitions"));
        assert!(with("a.rs", "nope", "log();").contains("no definition `nope`"));
        assert!(with("a.rs", "Store::put", "nothing").contains("not found inside `Store::put`"));
        assert!(with("a.rs", "Store::put", "impl Queue").contains("appears outside it"));
        assert!(with("a.txt", "put", "log();").contains("registered Tree-sitter grammar"));
    }

    #[test]
    fn unique_match_applies() {
        let (out, applied) = apply_hunks("f", "a\nb\nc\n", &[hunk("b", "B", None)]).unwrap();
        assert_eq!(out, "a\nB\nc\n");
        assert_eq!(applied[0].start_line, Some(2));
    }

    #[test]
    fn ambiguous_match_needs_hint() {
        let text = "x\ny\nx\n";
        assert!(apply_hunks("f", text, &[hunk("x", "z", None)]).is_err());
        let (out, _) = apply_hunks("f", text, &[hunk("x", "z", Some(3))]).unwrap();
        assert_eq!(out, "x\ny\nz\n");
    }

    #[test]
    fn crlf_files_accept_lf_hunks() {
        let (out, _) = apply_hunks("f", "a\r\nb\r\nc\r\n", &[hunk("a\nb", "a\nB", None)]).unwrap();
        assert_eq!(out, "a\r\nB\r\nc\r\n");
    }

    #[test]
    fn failures_explain_themselves() {
        let err = apply_hunks("f", "  foo(1)\n", &[hunk("foo(2)\n", "", None)]).unwrap_err();
        assert!(err.to_string().contains("not found"));
        assert!(apply_hunks("f", "a", &[hunk("", "x", None)]).is_err());
    }

    #[test]
    fn empty_file_can_be_patched() {
        let (out, applied) = apply_hunks("f", "", &[hunk("", "hello\nworld\n", None)]).unwrap();
        assert_eq!(out, "hello\nworld\n");
        assert_eq!(applied[0].start_line, Some(1));
        let back = apply_hunks_reverse("f", &out, &applied).unwrap();
        assert_eq!(back, "");
    }

    #[test]
    fn start_line_uses_pre_patch_line_numbers_across_line_shifts() {
        let text = "header\ndup\nmiddle\ndup\n";
        let hunks = [
            hunk("header\n", "h1\nh2\nh3\nh4\n", Some(1)),
            hunk("dup\n", "TARGET\n", Some(4)),
        ];
        let (out, applied) = apply_hunks("f", text, &hunks).unwrap();
        assert_eq!(out, "h1\nh2\nh3\nh4\ndup\nmiddle\nTARGET\n");
        assert_eq!(applied[0].start_line, Some(1));
        assert_eq!(applied[1].start_line, Some(4));

        let restored = apply_hunks_reverse("f", &out, &applied).unwrap();
        assert_eq!(restored, text);
    }
}
