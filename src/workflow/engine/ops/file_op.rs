//! The file operations of `workflow_write_files`. The same type records each turn's net changes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::LineNumber;
use crate::common::params::flex_line_number;

/// A replacement inside a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PatchHunk {
    /// Optional 1-based line number in the pre-patch file. Used to disambiguate
    /// if `old_text` matches in multiple places.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "line",
        alias = "lines",
        deserialize_with = "flex_line_number"
    )]
    pub start_line: Option<LineNumber>,
    /// Exact text to replace. Must match once (or be disambiguated by `start_line`).
    pub old_text: String,
    /// Replacement text (may be empty to delete).
    pub new_text: String,
    /// A definition (`name`, `Type::name`, `Class.name`) in a file of a language with a registered
    /// Tree-sitter grammar. `old_text` is searched only inside it, so identical code in other
    /// definitions cannot match. Applied hunks never keep it; the turn ledger records `start_line`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
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
    /// Replace the definition `symbol` (`name`, `Type::name`, `Class.name`; Rust and Python) with
    /// `content`, moved to the definition's indentation. Doc comments and attributes above the
    /// definition stay unless `content` starts with its own. Input only: the turn ledger records
    /// the resulting hunks.
    ReplaceSymbol {
        path: String,
        symbol: String,
        content: String,
    },
    /// Insert `content` above the definition `symbol`, above its doc comments and attributes,
    /// followed by a blank line. Input only.
    InsertBeforeSymbol {
        path: String,
        symbol: String,
        content: String,
    },
    /// Insert `content` below the definition `symbol`, after a blank line. Input only.
    InsertAfterSymbol {
        path: String,
        symbol: String,
        content: String,
    },
    /// Rename the identifiers of `symbol` (`name`, or `Type::name` to keep to that type) to
    /// `new_name` in the Rust and Python files below `path` (default: the whole project, without
    /// gitignored files). Comments, string literals and uses through an import alias are left
    /// alone. A qualified `symbol` keeps to identifiers inside that type and lines that name it,
    /// so a call such as `store.put()` is not renamed; the result counts the same-named
    /// identifiers left alone. Input only: the turn ledger records the resulting hunks.
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
