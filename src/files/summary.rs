//! One-line descriptions of file operations, and their unified-diff rendering.

use crate::files::diff::{hunk_body, hunk_stats};
use crate::params::{FileOp, PatchHunk};

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
    }
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
            };
            out.push_str(&hunk_body(&hunk));
        }
        FileOp::Patch { hunks, .. } => {
            let mut shift = 0i64;
            for hunk in hunks {
                let old_n = hunk.old_text.split_inclusive('\n').count() as i64;
                let new_n = hunk.new_text.split_inclusive('\n').count() as i64;
                let old_start = hunk.start_line.unwrap_or(1) as i64;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_are_one_line_each() {
        let create = FileOp::Create {
            path: "a.rs".into(),
            content: "x\ny\n".into(),
        };
        assert_eq!(op_summary(&create), "Create a.rs (2 lines)");
        let mv = FileOp::Move {
            path: "a".into(),
            to_path: "b".into(),
        };
        assert_eq!(op_summary(&mv), "Move a → b");
        assert_eq!(op_summary(&FileOp::Delete { path: "a".into() }), "Delete a");
    }

    #[test]
    fn patch_renders_as_unified_diff_with_shifted_new_lines() {
        let hunk = |line: u64, old: &str, new: &str| PatchHunk {
            start_line: Some(line),
            old_text: old.into(),
            new_text: new.into(),
        };
        let op = FileOp::Patch {
            path: "a.rs".into(),
            hunks: vec![hunk(2, "b\n", "b\nc\n"), hunk(10, "x\n", "y\n")],
        };
        assert_eq!(
            op_diff(&op),
            "Patch a.rs +2 -1 at 2,10\n@@ -2,1 +2,2 @@\n b\n+c\n@@ -10,1 +11,1 @@\n-x\n+y\n"
        );
    }

    #[test]
    fn create_lists_content_as_added_lines() {
        let op = FileOp::Create {
            path: "a.txt".into(),
            content: "one\ntwo".into(),
        };
        assert_eq!(
            op_diff(&op),
            "Create a.txt (2 lines)\n+one\n+two\n\\ No newline at end of file\n"
        );
    }
}
