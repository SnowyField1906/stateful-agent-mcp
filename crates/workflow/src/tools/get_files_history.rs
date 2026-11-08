//! `workflow_get_files_history`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Project;
use crate::common::params::opt_string_or_vec;
use crate::common::pathspec::{matches_pathspec, pathspecs};
use crate::common::{AgentError, Result, ServerConfig};
use crate::engine::ops::FileOp;
use crate::engine::ops::summary::op_summary;
use crate::engine::selector::Sel;

pub const DESCRIPTION: &str = "List which turns changed which files, newest first, without content (`workflow_get_files_diff` gives content). `implicit_id` limits the turns, `implicit_paths` the files; renames are followed.";

/// Parameters for `workflow_get_files_history`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GetFilesHistoryParams {
    /// Turn, session or range to inspect (default: all turns).
    #[serde(default)]
    pub implicit_id: Option<String>,
    /// Limit to these files or directory prefixes; renames are followed.
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    pub implicit_paths: Option<Vec<String>>,
}

/// Which turns changed which files, newest first. Content is omitted; see `get_files_diff`.
pub fn get_files_history(
    project: &Project,
    cfg: &ServerConfig,
    p: GetFilesHistoryParams,
) -> Result<Value> {
    let limit = cfg.default_limit.max(1);
    let flat = project.flat_keys();
    let (lo, hi) = match p.implicit_id.as_deref() {
        Some(raw) => match project.parse_selector(&flat, raw)? {
            Sel::Turns { lo, hi, .. } => (lo, hi),
            _ => {
                return Err(AgentError::invalid(
                    "implicit_id",
                    "expected a turn, session or range",
                ));
            }
        },
        None => (0, flat.len() - 1),
    };
    let mut tracked = pathspecs(p.implicit_paths.as_deref(), &project.root)?;
    let mut turns: Vec<Value> = Vec::new();
    for key in flat[lo..=hi].iter().rev() {
        let Some(turn) = project.turn(*key) else {
            continue;
        };
        let mut updates: Vec<String> = Vec::new();
        for op in &turn.updates {
            if !op.in_pathspecs(&tracked) {
                continue;
            }
            updates.push(op_summary(op));
            // Follow renames backwards: from now on the old path is of interest too.
            if let FileOp::Move { path, to_path } = op
                && !tracked.is_empty()
                && tracked.iter().any(|s| matches_pathspec(to_path, s))
            {
                tracked.push(path.clone());
            }
        }
        if !updates.is_empty() {
            let mut entry = turn.header();
            entry.insert("updates".into(), json!(updates));
            turns.push(Value::Object(entry));
        }
    }
    let total = turns.len();
    turns.truncate(limit);
    let mut out = json!({ "turns": turns });
    if total > limit {
        out["omitted"] = json!(total - limit);
    }
    Ok(out)
}
