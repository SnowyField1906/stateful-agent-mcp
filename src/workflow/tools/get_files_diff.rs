//! `workflow_get_files_diff`.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::common::params::opt_string_or_vec;
use crate::common::pathspec::{in_pathspecs, pathspecs};
use crate::common::{AgentError, Result};
use crate::workflow::Project;
use crate::workflow::engine::ops::net_ops::net_ops;
use crate::workflow::engine::ops::summary::op_diff;
use crate::workflow::engine::ops::{FileOp, Tree};
use crate::workflow::engine::selector::Sel;
use crate::workflow::engine::session::model::TurnKey;

pub const DESCRIPTION: &str = "Show the net file changes of a turn, session, range or update (`implicit_id`) as a unified diff. A reversed range (`01.05..01.02`) gives the inverse diff. A deleted file shows no content; restore it with `Recover`. `implicit_paths` limits the result to files or directories.";

/// Parameters for `workflow_get_files_diff`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetFilesDiffParams {
    /// Turn (`01.02`), session (`01`), range (`01.02..01.05`; a reversed range gives the inverse
    /// diff), `HEAD`, or update ID (`01.02.update.src/a.rs`).
    pub implicit_id: String,
    /// Limit to these files or directory prefixes.
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    pub implicit_paths: Option<Vec<String>>,
}

/// Net file changes of the selected turns as a unified diff, one block per operation.
pub fn get_files_diff(project: &Project, p: GetFilesDiffParams) -> Result<String> {
    let id = &p.implicit_id;
    let paths = p.implicit_paths.as_deref();
    let flat = project.flat_keys();
    let specs = pathspecs(paths, &project.root)?;
    let ops: Vec<FileOp> = match project.parse_selector(&flat, id)? {
        Sel::Update(key, path) => {
            let turn = project
                .turn(key)
                .ok_or_else(|| AgentError::NotFound(format!("turn {key}")))?;
            let op = turn
                .updates
                .iter()
                .find(|op| op.touches_path(&path))
                .ok_or_else(|| AgentError::NotFound(id.trim().to_string()))?;
            vec![op.clone()]
        }
        Sel::Turns { lo, hi, reversed } if lo == hi && !reversed => {
            let turn = project
                .turn(flat[lo])
                .ok_or_else(|| AgentError::NotFound(format!("turn {}", flat[lo])))?;
            turn.updates
                .iter()
                .filter(|op| op.in_pathspecs(&specs))
                .cloned()
                .collect()
        }
        Sel::Turns { lo, hi, reversed } => {
            let mut touched = BTreeSet::new();
            for key in &flat[lo..=hi] {
                touched.extend(project.touched_paths(*key));
            }
            let moves = range_moves(project, &flat[lo..=hi], reversed);
            let (mut before, mut after) = (Tree::new(), Tree::new());
            for path in touched {
                if !in_pathspecs(&path, &specs) {
                    continue;
                }
                before.insert(path.clone(), project.content_before(&flat, lo, &path)?);
                after.insert(path.clone(), project.content_before(&flat, hi + 1, &path)?);
            }
            if reversed {
                net_ops(&after, &before, &moves)
            } else {
                net_ops(&before, &after, &moves)
            }
        }
        _ => {
            return Err(AgentError::invalid(
                "implicit_id",
                "expected a turn, session, range or update ID",
            ));
        }
    };
    if ops.is_empty() {
        return Ok("No changes.\n".to_string());
    }
    Ok(ops.iter().map(op_diff).collect())
}

/// Net move origins (`to_path -> from_path`) across a slice of turns, inverted when `reversed`.
fn range_moves(project: &Project, keys: &[TurnKey], reversed: bool) -> BTreeMap<String, String> {
    let mut moves: BTreeMap<String, String> = BTreeMap::new();
    for key in keys {
        let Some(turn) = project.turn(*key) else {
            continue;
        };
        for op in &turn.updates {
            match op {
                FileOp::Move { path, to_path } => {
                    let orig = moves.remove(path).unwrap_or_else(|| path.clone());
                    if orig != *to_path {
                        moves.insert(to_path.clone(), orig);
                    }
                }
                FileOp::Delete { path } => {
                    moves.remove(path);
                }
                _ => {}
            }
        }
    }
    if reversed {
        moves.into_iter().map(|(to, from)| (from, to)).collect()
    } else {
        moves
    }
}
