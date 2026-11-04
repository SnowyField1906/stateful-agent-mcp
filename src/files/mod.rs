//! File-operation engine: path safety, atomic overlay, patching, diffing and net-change
//! computation.

pub mod diff;
pub mod net_ops;
pub mod overlay;
pub mod patch;
pub mod path;
pub mod summary;

use std::collections::BTreeMap;

/// File contents by project-relative path; `None` means the file does not exist.
pub type Tree = BTreeMap<String, Option<String>>;
