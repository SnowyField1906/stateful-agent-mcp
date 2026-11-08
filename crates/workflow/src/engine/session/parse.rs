//! Reading a session: the turns of one `.AGENTS/sessions/{SS}.md`, in the format of `format`.

use crate::common::{AgentError, Result};
use crate::engine::ops::net_ops::merge_update_ops;
use crate::engine::ops::{FileOp, PatchHunk};

use super::format::{
    FENCE, REF_TAG, ROLLED_OVER_TAG, SECTION_PREFIX, Section, TURN_PREFIX, USER_EXPLICIT_TAG,
    html_unescape, unprotect,
};
use super::model::{HandoverTaskRecord, NoteRecord, TurnKey, TurnRecord};

/// Parse one session file. `source` names the file in error messages.
/// Unparsable update blocks are an error: silently dropping them would lose history.
pub fn parse(session: u32, markdown: &str, source: &str) -> Result<Vec<TurnRecord>> {
    let lines: Vec<&str> = markdown.split('\n').collect();
    let mut turns = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(header) = lines[i].strip_prefix(TURN_PREFIX) else {
            i += 1;
            continue;
        };
        let start = i + 1;
        let end = lines[start..]
            .iter()
            .position(|line| line.starts_with(TURN_PREFIX))
            .map_or(lines.len(), |n| start + n);
        turns.push(parse_turn(session, header, &lines[start..end], source)?);
        i = end;
    }
    Ok(turns)
}

/// One turn: its `header` (after `## TURN `) and the lines up to the next turn.
fn parse_turn(session: u32, header: &str, body: &[&str], source: &str) -> Result<TurnRecord> {
    let Some((number, title)) = parse_header(header) else {
        return Err(AgentError::Corrupt {
            path: source.to_string(),
            reason: format!("bad turn header `{TURN_PREFIX}{header}`"),
        });
    };
    let mut turn = TurnRecord::new(TurnKey::new(session, number));
    turn.title = title;
    for (section, lines) in sections(body) {
        match section {
            Some(Section::Notes) => {
                turn.notes
                    .extend(parse_numbered_list(lines).iter().map(|s| parse_note(s)));
            }
            Some(Section::Tasks) => {
                turn.tasks
                    .extend(parse_numbered_list(lines).iter().map(|s| parse_task(s)));
            }
            Some(Section::Updates) => turn.updates.extend(parse_updates(lines, source, turn.key)?),
            None => {}
        }
    }
    if !turn.updates.is_empty() {
        turn.updates = merge_update_ops(turn.updates).map_err(|e| AgentError::Corrupt {
            path: source.to_string(),
            reason: format!("turn {} updates: {e}", turn.key),
        })?;
    }
    Ok(turn)
}

/// Split the body of a turn at its `### ` headings. Each part is the section its first line
/// opens (`None` for a line that is no known heading) and the lines after it; the first part
/// starts with the line right below the turn header.
fn sections<'a>(body: &'a [&'a str]) -> Vec<(Option<Section>, &'a [&'a str])> {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let start = i + 1;
        let end = body[start..]
            .iter()
            .position(|line| line.starts_with(SECTION_PREFIX))
            .map_or(body.len(), |n| start + n);
        parts.push((Section::of(body[i]), &body[start..end]));
        i = end;
    }
    parts
}

/// `01.02` or `01.02 — title`, exactly as `serialize_turn` writes it.
fn parse_header(rest: &str) -> Option<(u32, Option<String>)> {
    let rest = rest.trim();
    let (id, title) = match rest.split_once(" — ") {
        Some((id, title)) => (id, Some(title.trim())),
        None => (rest, None),
    };
    let key = TurnKey::parse(id)?;
    Some((
        key.turn,
        title.filter(|t| !t.is_empty()).map(str::to_string),
    ))
}

fn parse_numbered_list(lines: &[&str]) -> Vec<String> {
    let mut items = Vec::new();
    let mut current: Option<(usize, Vec<String>)> = None;

    for &line in lines {
        if let Some((indent, first)) = split_number_prefix(line) {
            if let Some((_, buf)) = current.take() {
                items.push(join_item(buf));
            }
            current = Some((indent, vec![first.to_string()]));
        } else if let Some((indent, buf)) = current.as_mut() {
            buf.push(strip_indent(line, *indent).to_string());
        }
    }
    if let Some((_, buf)) = current {
        items.push(join_item(buf));
    }
    items
}

/// Remove at most `width` leading spaces. Tabs and further spaces belong to the item text.
fn strip_indent(line: &str, width: usize) -> &str {
    let spaces = line.bytes().take(width).take_while(|&b| b == b' ').count();
    &line[spaces..]
}

/// `"12. text"` -> `(4, "text")`: the prefix width and the remaining text.
fn split_number_prefix(line: &str) -> Option<(usize, &str)> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || !line[digits..].starts_with(". ") {
        return None;
    }
    let width = digits + 2;
    Some((width, &line[width..]))
}

fn join_item(mut lines: Vec<String>) -> String {
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

fn parse_note(raw: &str) -> NoteRecord {
    let mut rest = raw;
    let mut ref_notes = Vec::new();
    if let Some(after) = rest.strip_prefix(REF_TAG)
        && let Some((ids, remainder)) = after.split_once(']')
    {
        ref_notes = ids
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        rest = remainder.strip_prefix(' ').unwrap_or(remainder);
    }
    NoteRecord {
        content: unprotect(rest).to_string(),
        ref_notes,
    }
}

fn parse_task(raw: &str) -> HandoverTaskRecord {
    let mut rest = raw;
    let mut user_explicit = false;
    let mut rolled_over_from = None;
    loop {
        if let Some(after) = rest.strip_prefix(USER_EXPLICIT_TAG) {
            user_explicit = true;
            rest = after.strip_prefix(' ').unwrap_or(after);
        } else if let Some(after) = rest.strip_prefix(ROLLED_OVER_TAG)
            && let Some((origin, remainder)) = after.split_once(']')
        {
            let origin = origin.trim();
            if !origin.is_empty() {
                rolled_over_from = Some(origin.to_string());
            }
            rest = remainder.strip_prefix(' ').unwrap_or(remainder);
        } else {
            break;
        }
    }
    HandoverTaskRecord {
        content: unprotect(rest).to_string(),
        user_explicit,
        rolled_over_from,
    }
}

/// Parse the `<details>` blocks of an `### Updates` section. Anything unreadable is an error:
/// silently dropping it would lose history.
fn parse_updates(body: &[&str], source: &str, key: TurnKey) -> Result<Vec<FileOp>> {
    let corrupt = |reason: String| AgentError::Corrupt {
        path: source.to_string(),
        reason: format!("turn {key} updates: {reason}"),
    };
    let mut ops = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let line = body[i].trim();
        i += 1;
        if line.is_empty() {
            continue;
        }
        if line != "<details>" {
            return Err(corrupt(format!("unexpected line `{line}`")));
        }
        let summary = body
            .get(i)
            .and_then(|l| {
                l.trim()
                    .strip_prefix("<summary>")?
                    .strip_suffix("</summary>")
            })
            .ok_or_else(|| corrupt("missing <summary>".into()))?;
        i += 1;
        let mut diff: &[&str] = &[];
        while i < body.len() && body[i].trim() != "</details>" {
            if body[i].trim_end() == "```diff" {
                let start = i + 1;
                i = start;
                while i < body.len() && body[i].trim_end() != FENCE {
                    i += 1;
                }
                if i >= body.len() {
                    return Err(corrupt("diff block is not closed".into()));
                }
                diff = &body[start..i];
            }
            i += 1;
        }
        if i >= body.len() {
            return Err(corrupt("<details> is not closed".into()));
        }
        i += 1;
        ops.push(
            parse_update(summary, diff)
                .ok_or_else(|| corrupt(format!("bad update `{summary}`")))?,
        );
    }
    Ok(ops)
}

/// The `<code>` texts of an update summary, unescaped, in order; empty when one is not closed.
fn summary_paths(summary: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut rest = summary;
    while let Some((_, after_open)) = rest.split_once("<code>") {
        let Some((path, after_close)) = after_open.split_once("</code>") else {
            return Vec::new();
        };
        paths.push(html_unescape(path));
        rest = after_close;
    }
    paths
}

/// `Verb <code>path</code> ...` plus its diff lines -> operation.
fn parse_update(summary: &str, diff: &[&str]) -> Option<FileOp> {
    let (verb, _) = summary.split_once(' ')?;
    let mut paths = summary_paths(summary).into_iter();
    let path = paths.next()?;
    match verb {
        "Create" => {
            let content = parse_hunks(diff).into_iter().next()?.new_text;
            Some(FileOp::Create { path, content })
        }
        "Patch" => Some(FileOp::Patch {
            path,
            hunks: parse_hunks(diff),
        }),
        "Move" => Some(FileOp::Move {
            path,
            to_path: paths.next()?,
        }),
        "Delete" => Some(FileOp::Delete { path }),
        "Recover" => Some(FileOp::Recover {
            id: path,
            to_path: paths.next(),
        }),
        _ => None,
    }
}

/// Inverse of `serialize`'s diff body: `-` old, `+` new, `...` between hunks,
/// `@@ N @@` start line, `\ No newline at end of file` drops the last newline.
fn parse_hunks(diff: &[&str]) -> Vec<PatchHunk> {
    let empty = || PatchHunk {
        start_line: None,
        old_text: String::new(),
        new_text: String::new(),
        symbol: None,
    };
    let mut hunks = Vec::new();
    let mut cur = empty();
    let mut last = '+';
    for &line in diff {
        if line == "..." {
            hunks.push(std::mem::replace(&mut cur, empty()));
        } else if let Some(n) = line.strip_prefix("@@ ").and_then(|l| l.strip_suffix(" @@")) {
            cur.start_line = n.trim().parse().ok();
        } else if line.starts_with("\\ No newline") {
            let buf = if last == '-' {
                &mut cur.old_text
            } else {
                &mut cur.new_text
            };
            if buf.ends_with('\n') {
                buf.pop();
            }
        } else if let Some(text) = line.strip_prefix('-') {
            cur.old_text.push_str(text);
            cur.old_text.push('\n');
            last = '-';
        } else if let Some(text) = line.strip_prefix('+') {
            cur.new_text.push_str(text);
            cur.new_text.push('\n');
            last = '+';
        }
    }
    hunks.push(cur);
    hunks
}
