//! Path normalization of the operations of a batch.

use std::path::Path;

use crate::common::Result;
use crate::common::pathspec::{normalize_dest, normalize_rel};
use crate::engine::ops::FileOp;
use crate::engine::ops::overlay::Overlay;

/// Validate and normalize every path of `op`: spelled as the disk stores it, or else as the files
/// that earlier operations of the batch created.
pub(super) fn normalize_op(op: &FileOp, root: &Path, overlay: &Overlay) -> Result<FileOp> {
    let norm = |p: &str| normalize_rel(p, root).map(|n| overlay.spell_like_pending(&n, true));
    let dest = |p: &str| normalize_dest(p, root).map(|n| overlay.spell_like_pending(&n, false));
    Ok(match op {
        FileOp::Create { path, content } => FileOp::Create {
            path: norm(path)?,
            content: content.clone(),
        },
        FileOp::Patch { path, hunks } => FileOp::Patch {
            path: norm(path)?,
            hunks: hunks.clone(),
        },
        FileOp::Delete { path } => FileOp::Delete { path: norm(path)? },
        FileOp::ReplaceSymbol {
            path,
            symbol,
            content,
        } => FileOp::ReplaceSymbol {
            path: norm(path)?,
            symbol: symbol.trim().to_string(),
            content: content.clone(),
        },
        FileOp::InsertBeforeSymbol {
            path,
            symbol,
            content,
        } => FileOp::InsertBeforeSymbol {
            path: norm(path)?,
            symbol: symbol.trim().to_string(),
            content: content.clone(),
        },
        FileOp::InsertAfterSymbol {
            path,
            symbol,
            content,
        } => FileOp::InsertAfterSymbol {
            path: norm(path)?,
            symbol: symbol.trim().to_string(),
            content: content.clone(),
        },
        FileOp::RenameSymbol {
            symbol,
            new_name,
            path,
        } => FileOp::RenameSymbol {
            symbol: symbol.trim().to_string(),
            new_name: new_name.trim().to_string(),
            path: match path.as_deref().map(str::trim) {
                None | Some("" | ".") => None,
                Some(p) => Some(norm(p)?),
            },
        },
        FileOp::Move { path, to_path } => FileOp::Move {
            path: norm(path)?,
            to_path: dest(to_path)?,
        },
        FileOp::Recover { id, to_path } => FileOp::Recover {
            id: id.trim().to_string(),
            to_path: to_path.as_deref().map(norm).transpose()?,
        },
    })
}
