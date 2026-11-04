//! File operation types: `write_files` input and the net updates recorded for each turn.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A replacement inside a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PatchHunk {
    /// Optional 1-based line number in the pre-patch file. Used to disambiguate
    /// if `old_text` matches in multiple places.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u64>,
    /// Exact text to replace. Must match once (or be disambiguated by `start_line`).
    pub old_text: String,
    /// Replacement text (may be empty to delete).
    pub new_text: String,
}

/// A file operation. Paths are relative to the project root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op")]
pub enum FileOp {
    /// Create a new file with `content`. Fails if the path already exists; use `Patch` to edit
    /// existing files.
    Create { path: String, content: String },
    /// Move or rename a file or directory. The destination must not exist.
    Move { path: String, to_path: String },
    /// Replace text in an existing file.
    Patch { path: String, hunks: Vec<PatchHunk> },
    /// Delete a file or a directory with its files.
    Delete { path: String },
    /// Restore files from a past turn. `id` is an update ID (`01.02.update.src/a.rs`, with
    /// optional `to_path`), a turn/session/range selector with `to_path` (restoring that file or
    /// directory prefix), or a turn/session/range selector without `to_path` (restoring all files
    /// touched in those turns).
    Recover {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_path: Option<String>,
    },
}

impl FileOp {
    /// The path an update ID is built from (destination for moves).
    pub fn primary_path(&self) -> &str {
        match self {
            FileOp::Create { path, .. } | FileOp::Patch { path, .. } | FileOp::Delete { path } => path,
            FileOp::Move { to_path, .. } => to_path,
            FileOp::Recover { id, to_path } => to_path
                .as_deref()
                .or_else(|| id.split_once(".update.").map(|(_, rel)| rel))
                .unwrap_or(id),
        }
    }

    /// Every path this operation reads or writes.
    pub fn paths(&self) -> Vec<&str> {
        match self {
            FileOp::Move { path, to_path } => vec![path, to_path],
            other => vec![other.primary_path()],
        }
    }

    pub fn touches_path(&self, target: &str) -> bool {
        self.paths().contains(&target)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteFilesParams {
    /// Operations, applied in order as one atomic batch. Each uses `{op: "Create", ...}` shape.
    pub ops: Vec<FileOp>,
}

#[cfg(test)]
mod tests {
    use super::FileOp;
    use serde_json::json;

    #[test]
    fn file_ops_use_inline_op_tag_for_input_and_output() {
        let value = json!({"op": "Create", "path": "a.txt", "content": "x"});
        let op: FileOp = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(op).unwrap(), value);
    }
}
