//! Lossless (de)serialization of `.AGENTS/sessions/{SS}.md`.
//!
//! ```text
//! ## TURN 01.01 — optional title
//!
//! ### Notes
//!
//! 1. [REF 01.01.note.01] Note text
//!    Continuation lines are indented to the item text.
//!
//! ### Updates
//!
//! <details>
//! <summary>Patch <code>src/a.rs</code> (+3 -1)</summary>
//!
//! ```diff
//! -old line
//! +new line
//! ...
//! ```
//!
//! </details>
//!
//! (one `<details>` per file operation; hunks of one patch are separated by `...`)
//!
//! ### Handover Tasks
//!
//! 1. [ROLLED-OVER 01.01.task.02] [USER-EXPLICIT] Task text
//! ```

use crate::common::{AgentError, Result};
use crate::workflow::engine::ops::net_ops::merge_update_ops;
use crate::workflow::engine::ops::{FileOp, PatchHunk};

use super::model::{HandoverTaskRecord, NoteRecord, TurnKey, TurnRecord};

const FENCE: &str = "```";

/// Item text that starts with one of these is escaped with a leading backslash, so that it is
/// not read back as metadata.
const NOTE_TAGS: &[&str] = &["[REF "];
const TASK_TAGS: &[&str] = &["[ROLLED-OVER", "[USER-EXPLICIT]"];

/// Serialize the turns of one session.
pub fn serialize(turns: &[TurnRecord]) -> String {
    if turns.is_empty() {
        return String::new();
    }
    let blocks: Vec<String> = turns.iter().map(serialize_turn).collect();
    format!("{}\n", blocks.join("\n\n"))
}

fn serialize_turn(turn: &TurnRecord) -> String {
    let mut out = match turn
        .title
        .as_deref()
        .map(one_line)
        .filter(|t| !t.is_empty())
    {
        Some(title) => format!("## TURN {} — {title}\n", turn.key),
        None => format!("## TURN {}\n", turn.key),
    };

    if !turn.notes.is_empty() {
        out.push_str("\n### Notes\n\n");
        out.push_str(&numbered_items(turn.notes.iter().map(|n| {
            let content = protect(&n.content, NOTE_TAGS);
            if n.ref_notes.is_empty() {
                content
            } else {
                format!("[REF {}] {content}", n.ref_notes.join(", "))
            }
        })));
        out.push('\n');
    }

    if !turn.updates.is_empty() {
        out.push_str("\n### Updates\n\n");
        for op in &turn.updates {
            out.push_str(&serialize_update(op));
        }
    }

    if !turn.tasks.is_empty() {
        out.push_str("\n### Handover Tasks\n\n");
        out.push_str(&numbered_items(turn.tasks.iter().map(|t| {
            let mut tags = String::new();
            if let Some(origin) = &t.rolled_over_from {
                tags.push_str(&format!("[ROLLED-OVER {origin}] "));
            }
            if t.user_explicit {
                tags.push_str("[USER-EXPLICIT] ");
            }
            format!("{tags}{}", protect(&t.content, TASK_TAGS))
        })));
        out.push('\n');
    }

    out.trim_end().to_string()
}

/// One update as a collapsible `<details>` block; patches and creates carry a git-style diff.
fn serialize_update(op: &FileOp) -> String {
    let (summary, diff) = match op {
        FileOp::Create { path, content } => {
            let (add, diff) = diff_side('+', content);
            (
                format!("Create <code>{}</code> (+{add})", html_escape(path)),
                Some(diff),
            )
        }
        FileOp::Patch { path, hunks } => {
            let (mut add, mut del) = (0, 0);
            let mut diff = String::new();
            for (n, hunk) in hunks.iter().enumerate() {
                if n > 0 {
                    diff.push_str("...\n");
                }
                if let Some(line) = hunk.start_line {
                    diff.push_str(&format!("@@ {line} @@\n"));
                }
                let (d, old) = diff_side('-', &hunk.old_text);
                let (a, new) = diff_side('+', &hunk.new_text);
                del += d;
                add += a;
                diff.push_str(&old);
                diff.push_str(&new);
            }
            (
                format!("Patch <code>{}</code> (+{add} -{del})", html_escape(path)),
                Some(diff),
            )
        }
        FileOp::Move { path, to_path } => (
            format!(
                "Move <code>{}</code> → <code>{}</code>",
                html_escape(path),
                html_escape(to_path)
            ),
            None,
        ),
        FileOp::Delete { path } => (format!("Delete <code>{}</code>", html_escape(path)), None),
        FileOp::Recover {
            id,
            to_path: Some(to),
        } => (
            format!(
                "Recover <code>{}</code> → <code>{}</code>",
                html_escape(id),
                html_escape(to)
            ),
            None,
        ),
        FileOp::Recover { id, to_path: None } => {
            (format!("Recover <code>{}</code>", html_escape(id)), None)
        }
        FileOp::RenameSymbol {
            symbol, new_name, ..
        } => (
            format!(
                "RenameSymbol <code>{}</code> → <code>{}</code>",
                html_escape(symbol),
                html_escape(new_name)
            ),
            None,
        ),
        FileOp::ReplaceSymbol { path, symbol, .. }
        | FileOp::InsertBeforeSymbol { path, symbol, .. }
        | FileOp::InsertAfterSymbol { path, symbol, .. } => (
            format!(
                "{} <code>{}</code> <code>{}</code>",
                op.verb(),
                html_escape(path),
                html_escape(symbol)
            ),
            None,
        ),
    };
    let mut out = format!("<details>\n<summary>{summary}</summary>\n");
    if let Some(diff) = diff {
        out.push_str(&format!("\n{FENCE}diff\n{diff}{FENCE}\n\n"));
    }
    out.push_str("</details>\n\n");
    out
}

/// Prefix every line of `text` with `prefix`; a last line without newline gets git's marker.
/// Returns the line count and the diff lines.
fn diff_side(prefix: char, text: &str) -> (usize, String) {
    let mut out = String::new();
    let mut count = 0;
    for line in text.split_inclusive('\n') {
        count += 1;
        out.push(prefix);
        out.push_str(line.strip_suffix('\n').unwrap_or(line));
        out.push('\n');
        if !line.ends_with('\n') {
            out.push_str("\\ No newline at end of file\n");
        }
    }
    (count, out)
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// True when `s` begins with one or more `\` characters followed immediately by `[`.
fn starts_with_escaped_bracket(s: &str) -> bool {
    let slashes = s.bytes().take_while(|&b| b == b'\\').count();
    slashes > 0 && s.as_bytes().get(slashes) == Some(&b'[')
}

/// Prefix a backslash when `content` could be mistaken for a tag or for an escape.
fn protect(content: &str, tags: &[&str]) -> String {
    if starts_with_escaped_bracket(content) || tags.iter().any(|t| content.starts_with(t)) {
        format!("\\{content}")
    } else {
        content.to_string()
    }
}

/// Inverse of [`protect`].
fn unprotect(text: &str) -> &str {
    if starts_with_escaped_bracket(text) {
        &text[1..]
    } else {
        text
    }
}

fn numbered_items(items: impl Iterator<Item = String>) -> String {
    items
        .enumerate()
        .map(|(i, text)| numbered_item(i + 1, &text))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn numbered_item(number: usize, text: &str) -> String {
    let head = format!("{number}. ");
    let indent = " ".repeat(head.len());
    let mut lines = text.lines();
    let mut out = format!("{head}{}", lines.next().unwrap_or(""));
    for line in lines {
        out.push('\n');
        if !line.is_empty() {
            out.push_str(&indent);
            out.push_str(line);
        }
    }
    out
}

#[derive(Clone, Copy)]
enum Section {
    Notes,
    Updates,
    Tasks,
}

impl Section {
    fn of(heading: &str) -> Option<Self> {
        match heading.trim() {
            "### Notes" => Some(Self::Notes),
            "### Updates" => Some(Self::Updates),
            "### Handover Tasks" => Some(Self::Tasks),
            _ => None,
        }
    }
}

/// Parse one session file. `source` names the file in error messages.
/// Unparsable update blocks are an error: silently dropping them would lose history.
pub fn parse(session: u32, markdown: &str, source: &str) -> Result<Vec<TurnRecord>> {
    let lines: Vec<&str> = markdown.split('\n').collect();
    let mut turns: Vec<TurnRecord> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let Some(header) = lines[i].strip_prefix("## TURN ") else {
            i += 1;
            continue;
        };
        let Some((turn_num, title)) = parse_header(header) else {
            return Err(AgentError::Corrupt {
                path: source.to_string(),
                reason: format!("bad turn header `## TURN {header}`"),
            });
        };
        let mut turn = TurnRecord::new(TurnKey::new(session, turn_num));
        turn.title = title;
        i += 1;

        while i < lines.len() && !lines[i].starts_with("## TURN ") {
            let section = Section::of(lines[i]);
            i += 1;
            let start = i;
            while i < lines.len()
                && !lines[i].starts_with("### ")
                && !lines[i].starts_with("## TURN ")
            {
                i += 1;
            }
            let body = &lines[start..i];
            match section {
                Some(Section::Notes) => {
                    turn.notes
                        .extend(parse_numbered_list(body).iter().map(|s| parse_note(s)));
                }
                Some(Section::Tasks) => {
                    turn.tasks
                        .extend(parse_numbered_list(body).iter().map(|s| parse_task(s)));
                }
                Some(Section::Updates) => {
                    turn.updates.extend(parse_updates(body, source, turn.key)?)
                }
                None => {}
            }
        }
        if !turn.updates.is_empty() {
            turn.updates = merge_update_ops(turn.updates).map_err(|e| AgentError::Corrupt {
                path: source.to_string(),
                reason: format!("turn {} updates: {e}", turn.key),
            })?;
        }
        turns.push(turn);
    }
    Ok(turns)
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
    if let Some(after) = rest.strip_prefix("[REF ")
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
        if let Some(after) = rest.strip_prefix("[USER-EXPLICIT]") {
            user_explicit = true;
            rest = after.strip_prefix(' ').unwrap_or(after);
        } else if let Some(after) = rest.strip_prefix("[ROLLED-OVER")
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

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

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

/// Inverse of `serialize_update`'s diff body: `-` old, `+` new, `...` between hunks,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut turn = TurnRecord::new(TurnKey::new(1, 2));
        turn.title = Some("Fix parser".into());
        turn.notes = vec![
            NoteRecord {
                content: "first line\n\n### not a heading\n| a | b |".into(),
                ref_notes: vec!["01.01.note.01".into()],
            },
            NoteRecord {
                content: "second".into(),
                ref_notes: vec![],
            },
        ];
        turn.updates = vec![FileOp::Create {
            path: "a.txt".into(),
            content: "x\n```\ny".into(),
        }];
        turn.tasks = vec![HandoverTaskRecord {
            content: "continue".into(),
            user_explicit: true,
            rolled_over_from: Some("01.01.task.03".into()),
        }];

        let md = serialize(std::slice::from_ref(&turn));
        let parsed = parse(1, &md, "test").unwrap();
        assert_eq!(parsed.len(), 1);
        let p = &parsed[0];
        assert_eq!(p.key, turn.key);
        assert_eq!(p.title, turn.title);
        assert_eq!(p.notes, turn.notes);
        assert_eq!(p.updates, turn.updates);
        assert_eq!(p.tasks, turn.tasks);
    }

    #[test]
    fn whitespace_and_tag_like_text_survive() {
        let mut turn = TurnRecord::new(TurnKey::new(1, 1));
        turn.notes = vec![
            NoteRecord {
                content: "  indented first\n\tTab line\n    four spaces\n   \nlast".into(),
                ref_notes: vec![],
            },
            NoteRecord {
                content: "[REF 01.01.note.01] looks like a tag".into(),
                ref_notes: vec![],
            },
            NoteRecord {
                content: "\\[escaped".into(),
                ref_notes: vec!["01.01.note.01".into()],
            },
        ];
        turn.tasks = vec![HandoverTaskRecord {
            content: "[USER-EXPLICIT] not a flag".into(),
            user_explicit: false,
            rolled_over_from: None,
        }];
        let parsed = parse(1, &serialize(std::slice::from_ref(&turn)), "test").unwrap();
        assert_eq!(parsed[0].notes, turn.notes);
        assert_eq!(parsed[0].tasks, turn.tasks);
    }

    #[test]
    fn duplicate_sections_are_merged() {
        let md = "## TURN 01.01\n\n### Notes\n\n1. a\n\n### Handover Tasks\n\n1. t\n\n### Notes\n\n1. b\n\n### Handover Tasks\n\n1. u\n";
        let turn = &parse(1, md, "test").unwrap()[0];
        let notes: Vec<_> = turn.notes.iter().map(|n| n.content.as_str()).collect();
        let tasks: Vec<_> = turn.tasks.iter().map(|t| t.content.as_str()).collect();
        assert_eq!(notes, ["a", "b"]);
        assert_eq!(tasks, ["t", "u"]);
        let again = &parse(1, &serialize(std::slice::from_ref(turn)), "test").unwrap()[0];
        assert_eq!(again.notes, turn.notes);
    }

    #[test]
    fn updates_round_trip_as_diff_dropdowns() {
        let key = TurnKey::new(1, 1);
        let ops = vec![
            FileOp::Create {
                path: "a.txt".into(),
                content: "x\n```\n...\n+y".into(),
            },
            FileOp::Create {
                path: "empty.txt".into(),
                content: String::new(),
            },
            FileOp::Patch {
                path: "b.rs".into(),
                hunks: vec![
                    PatchHunk {
                        start_line: Some(3),
                        old_text: "old\n".into(),
                        new_text: "new\nmore\n".into(),
                        symbol: None,
                    },
                    PatchHunk {
                        start_line: None,
                        old_text: "tail".into(),
                        new_text: String::new(),
                        symbol: None,
                    },
                    PatchHunk {
                        start_line: Some(9),
                        old_text: String::new(),
                        new_text: "ins".into(),
                        symbol: None,
                    },
                ],
            },
            FileOp::Move {
                path: "c.txt".into(),
                to_path: "d.txt".into(),
            },
            FileOp::Delete {
                path: "e.txt".into(),
            },
            FileOp::Recover {
                id: "01.00.update.f.txt".into(),
                to_path: None,
            },
            FileOp::Recover {
                id: "01.00".into(),
                to_path: Some("g".into()),
            },
            FileOp::Delete {
                path: "dir/<&name>.rs".into(),
            },
        ];
        let md: String = ops.iter().map(serialize_update).collect();
        assert!(md.contains("<summary>Create <code>a.txt</code> (+4)</summary>"));
        assert!(md.contains("<summary>Patch <code>b.rs</code> (+3 -2)</summary>"));
        assert!(md.contains("```diff\n@@ 3 @@\n-old\n+new\n+more\n...\n-tail\n\\ No newline at end of file\n...\n@@ 9 @@\n+ins\n\\ No newline at end of file\n```"));
        assert!(md.contains("<summary>Move <code>c.txt</code> → <code>d.txt</code></summary>"));
        assert!(md.contains("<summary>Delete <code>dir/&lt;&amp;name&gt;.rs</code></summary>"));
        let lines: Vec<&str> = md.split('\n').collect();
        assert_eq!(parse_updates(&lines, "test", key).unwrap(), ops);
    }

    #[test]
    fn crlf_content_survives() {
        let key = TurnKey::new(1, 1);
        let ops = vec![FileOp::Create {
            path: "w.txt".into(),
            content: "a\r\nb\r\n".into(),
        }];
        let md: String = ops.iter().map(serialize_update).collect();
        let lines: Vec<&str> = md.split('\n').collect();
        assert_eq!(parse_updates(&lines, "test", key).unwrap(), ops);
    }

    #[test]
    fn corrupt_updates_are_an_error() {
        for md in [
            "## TURN 01.01\n\n### Updates\n\n<details>\n<summary>Create `a`</summary>\n\n```diff\n+x\n",
            "## TURN 01.01\n\n### Updates\n\nstray text\n",
            "## TURN 01.01\n\n### Updates\n\n<details>\n<summary>Bogus `a`</summary>\n</details>\n",
            "## TURN 01.01\n\n### Updates\n\n<details>\n<summary>Delete `old.rs`</summary>\n</details>\n",
            "## TURN 01.01\n\n### Updates\n\n```json\n[]\n```\n",
            "## TURN nonsense\n",
        ] {
            assert!(
                matches!(parse(1, md, "test"), Err(AgentError::Corrupt { .. })),
                "{md}"
            );
        }
    }
}
