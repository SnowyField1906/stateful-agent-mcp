//! File-operation engine: the operation types, atomic overlay, patching, diffing and net-change
//! computation.

use std::collections::BTreeMap;

mod diagnose;
pub mod diff;
pub(crate) mod elide;
pub mod file_op;
pub mod net_ops;
pub mod overlay;
pub mod patch;
mod rebase;
pub mod rename;
mod repair;
pub mod summary;
pub mod symbol_edit;

pub use file_op::{FileOp, PatchHunk};

/// File contents by project-relative path; `None` means the file does not exist.
pub type Tree = BTreeMap<String, Option<String>>;
