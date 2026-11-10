//! The file operations of `workflow_write_files`. The same type records each turn's net changes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::LineNumber;
use crate::common::params::flex_line_number;

/// A replacement inside a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PatchHunk {
    /// 1-based line in the pre-patch file; picks one of several matches of `old_text`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "line",
        alias = "lines",
        deserialize_with = "flex_line_number"
    )]
    pub start_line: Option<LineNumber>,
    /// Exact text to replace; it must match once.
    pub old_text: String,
    /// Replacement text (may be empty to delete).
    pub new_text: String,
    /// Definition (`name`, `Type::name`) that `old_text` is searched in.
    // Applied hunks never keep it; the turn ledger records `start_line`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

// The schema descriptions below stay short: the `workflow_write_files` description holds the full
// text of every operation, and each client lists this schema with every tool that carries it.

/// A file operation. Paths are relative to the project root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op")]
pub enum FileOp {
    /// Create a new file; fails if it exists.
    Create { path: String, content: String },
    /// Move or rename a file or directory.
    Move { path: String, to_path: String },
    /// Replace text in an existing file.
    Patch { path: String, hunks: Vec<PatchHunk> },
    /// Delete a file or a directory with its files.
    Delete { path: String },
    /// Restore files from a past turn or update.
    Recover {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_path: Option<String>,
    },
    /// Replace a whole definition. Input only: the turn ledger records the resulting hunks.
    ReplaceSymbol {
        path: String,
        symbol: String,
        content: String,
    },
    /// Insert above a definition and its docs. Input only.
    InsertBeforeSymbol {
        path: String,
        symbol: String,
        content: String,
    },
    /// Insert below a definition. Input only.
    InsertAfterSymbol {
        path: String,
        symbol: String,
        content: String,
    },
    /// Rename the identifiers of a definition. Input only.
    RenameSymbol {
        symbol: String,
        new_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
}

impl FileOp {
    /// The name of the operation, as written in its `op` field.
    pub fn verb(&self) -> &'static str {
        match self {
            FileOp::Create { .. } => "Create",
            FileOp::Move { .. } => "Move",
            FileOp::Patch { .. } => "Patch",
            FileOp::Delete { .. } => "Delete",
            FileOp::Recover { .. } => "Recover",
            FileOp::ReplaceSymbol { .. } => "ReplaceSymbol",
            FileOp::InsertBeforeSymbol { .. } => "InsertBeforeSymbol",
            FileOp::InsertAfterSymbol { .. } => "InsertAfterSymbol",
            FileOp::RenameSymbol { .. } => "RenameSymbol",
        }
    }

    /// The path an update ID is built from (destination for moves).
    pub fn primary_path(&self) -> &str {
        match self {
            FileOp::Create { path, .. }
            | FileOp::Patch { path, .. }
            | FileOp::Delete { path }
            | FileOp::ReplaceSymbol { path, .. }
            | FileOp::InsertBeforeSymbol { path, .. }
            | FileOp::InsertAfterSymbol { path, .. } => path,
            FileOp::Move { to_path, .. } => to_path,
            FileOp::RenameSymbol { path, .. } => path.as_deref().unwrap_or("."),
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

    /// True when `specs` is empty or one of them matches a path of this operation.
    pub fn in_pathspecs(&self, specs: &[String]) -> bool {
        self.paths()
            .iter()
            .any(|p| crate::common::pathspec::in_pathspecs(p, specs))
    }
}
