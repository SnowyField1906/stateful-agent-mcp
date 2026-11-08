//! Writing a session: the turns of one `.AGENTS/sessions/{SS}.md`, in the format of `format`.

use crate::engine::ops::{FileOp, PatchHunk};

use super::format::{
    FENCE, NOTE_TAGS, REF_TAG, ROLLED_OVER_TAG, Section, TASK_TAGS, TURN_PREFIX, USER_EXPLICIT_TAG,
    html_escape, protect,
};
use super::model::{HandoverTaskRecord, NoteRecord, TurnRecord};

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
        Some(title) => format!("{TURN_PREFIX}{} — {title}\n", turn.key),
        None => format!("{TURN_PREFIX}{}\n", turn.key),
    };

    if !turn.notes.is_empty() {
        start_section(&mut out, Section::Notes);
        out.push_str(&numbered_items(turn.notes.iter().map(note_text)));
        out.push('\n');
    }

    if !turn.updates.is_empty() {
        start_section(&mut out, Section::Updates);
        for op in &turn.updates {
            out.push_str(&serialize_update(op));
        }
    }

    if !turn.tasks.is_empty() {
        start_section(&mut out, Section::Tasks);
        out.push_str(&numbered_items(turn.tasks.iter().map(task_text)));
        out.push('\n');
    }

    out.trim_end().to_string()
}

/// The heading of `section`, set off by a blank line before and after.
fn start_section(out: &mut String, section: Section) {
    out.push_str(&format!("\n{}\n\n", section.heading()));
}

/// The text of a note, led by its `[REF ..]` tag when it links other notes.
fn note_text(note: &NoteRecord) -> String {
    let content = protect(&note.content, NOTE_TAGS);
    if note.ref_notes.is_empty() {
        content
    } else {
        format!("{REF_TAG}{}] {content}", note.ref_notes.join(", "))
    }
}

/// The text of a task, led by its `[ROLLED-OVER ..]` and `[USER-EXPLICIT]` tags.
fn task_text(task: &HandoverTaskRecord) -> String {
    let mut tags = String::new();
    if let Some(origin) = &task.rolled_over_from {
        tags.push_str(&format!("{ROLLED_OVER_TAG} {origin}] "));
    }
    if task.user_explicit {
        tags.push_str(&format!("{USER_EXPLICIT_TAG} "));
    }
    format!("{tags}{}", protect(&task.content, TASK_TAGS))
}

/// `text` in `<code>`, escaped for HTML.
fn code(text: &str) -> String {
    format!("<code>{}</code>", html_escape(text))
}

/// One update as a collapsible `<details>` block; patches and creates carry a git-style diff.
fn serialize_update(op: &FileOp) -> String {
    let (summary, diff) = match op {
        FileOp::Create { path, content } => {
            let (add, diff) = diff_side('+', content);
            (format!("Create {} (+{add})", code(path)), Some(diff))
        }
        FileOp::Patch { path, hunks } => {
            let (add, del, diff) = patch_diff(hunks);
            (format!("Patch {} (+{add} -{del})", code(path)), Some(diff))
        }
        FileOp::Move { path, to_path } => {
            (format!("Move {} → {}", code(path), code(to_path)), None)
        }
        FileOp::Delete { path } => (format!("Delete {}", code(path)), None),
        FileOp::Recover {
            id,
            to_path: Some(to),
        } => (format!("Recover {} → {}", code(id), code(to)), None),
        FileOp::Recover { id, to_path: None } => (format!("Recover {}", code(id)), None),
        FileOp::RenameSymbol {
            symbol, new_name, ..
        } => (
            format!("RenameSymbol {} → {}", code(symbol), code(new_name)),
            None,
        ),
        FileOp::ReplaceSymbol { path, symbol, .. }
        | FileOp::InsertBeforeSymbol { path, symbol, .. }
        | FileOp::InsertAfterSymbol { path, symbol, .. } => (
            format!("{} {} {}", op.verb(), code(path), code(symbol)),
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

/// The diff body of a patch with its added and deleted line counts. Hunks are separated by `...`
/// and a hunk with a start line begins with `@@ N @@`.
fn patch_diff(hunks: &[PatchHunk]) -> (usize, usize, String) {
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
    (add, del, diff)
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
