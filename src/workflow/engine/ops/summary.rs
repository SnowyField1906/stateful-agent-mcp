//! One-line descriptions of file operations, and their unified-diff rendering.

use super::diff::{hunk_body, hunk_stats};
use super::{FileOp, PatchHunk};

/// Summary line of `op`: `Create a.rs (12 lines)`, `Patch a.rs +3 -1 at 12,40`, `Move a → b`,
/// `Delete a`, `Recover id → path`. Callers already know the turn, so no update ID is included;
/// it is `{turn}.update.{path}`.
pub fn op_summary(op: &FileOp) -> String {
    match op {
        FileOp::Create { path, content } => {
            format!("Create {path} ({} lines)", content.lines().count())
        }
        FileOp::Patch { path, hunks } => {
            let (added, removed) = hunks
                .iter()
                .map(hunk_stats)
                .fold((0, 0), |a, s| (a.0 + s.0, a.1 + s.1));
            let at: Vec<String> = hunks
                .iter()
                .filter_map(|h| h.start_line)
                .map(|line| line.to_string())
                .collect();
            let mut out = format!("Patch {path} +{added} -{removed}");
            if !at.is_empty() {
                out.push_str(&format!(" at {}", at.join(",")));
            }
            out
        }
        FileOp::Move { path, to_path } => format!("Move {path} → {to_path}"),
        FileOp::Delete { path } => format!("Delete {path}"),
        FileOp::Recover { id, to_path } => {
            format!(
                "Recover {id} → {}",
                to_path.as_deref().unwrap_or_else(|| op.primary_path())
            )
        }
        FileOp::RenameSymbol {
            symbol,
            new_name,
            path,
        } => {
            let mut out = format!("RenameSymbol {symbol} → {new_name}");
            if let Some(path) = path {
                out.push_str(&format!(" in {path}"));
            }
            out
        }
        FileOp::ReplaceSymbol { path, symbol, .. }
        | FileOp::InsertBeforeSymbol { path, symbol, .. }
        | FileOp::InsertAfterSymbol { path, symbol, .. } => {
            format!("{} {path} {symbol}", op.verb())
        }
    }
}

/// Summary line of a symbol operation after it was applied as `hunk`:
/// `ReplaceSymbol a.rs Store::put +3 -1 at 12`.
pub fn symbol_edit_summary(op: &FileOp, hunk: &PatchHunk) -> String {
    let (added, removed) = hunk_stats(hunk);
    let mut out = format!("{} +{added} -{removed}", op_summary(op));
    if let Some(line) = hunk.start_line {
        out.push_str(&format!(" at {line}"));
    }
    out
}

/// The summary line followed by the content change as a unified diff. A `Create` lists its
/// content as added lines; a `Patch` gives one `@@ -old,n +new,m @@` header per hunk, with `old`
/// counted in the file before the operation and `new` after the earlier hunks were applied.
pub fn op_diff(op: &FileOp) -> String {
    let mut out = format!("{}\n", op_summary(op));
    match op {
        FileOp::Create { content, .. } => {
            let hunk = PatchHunk {
                start_line: None,
                old_text: String::new(),
                new_text: content.clone(),
                symbol: None,
            };
            out.push_str(&hunk_body(&hunk));
        }
        FileOp::Patch { hunks, .. } => {
            let mut shift = 0i64;
            for hunk in hunks {
                let old_n = hunk.old_text.split_inclusive('\n').count() as i64;
                let new_n = hunk.new_text.split_inclusive('\n').count() as i64;
                let old_start = hunk.start_line.map_or(1, |l| l.get()) as i64;
                let new_start = (old_start + shift).max(0);
                out.push_str(&format!(
                    "@@ -{old_start},{old_n} +{new_start},{new_n} @@\n"
                ));
                out.push_str(&hunk_body(hunk));
                shift += new_n - old_n;
            }
        }
        _ => {}
    }
    out
}
