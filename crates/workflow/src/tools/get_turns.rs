//! `workflow_get_turns`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Project;
use crate::common::params::{PageParams, opt_string_or_vec};
use crate::common::{Result, ServerConfig};
use crate::engine::session::model::TurnRecord;

pub const DESCRIPTION: &str = "List turns as a timeline: ID, title, counts of notes, tasks and file updates, and up to 8 touched paths. Without `entries` the newest page is shown. Use it to find a turn before reading its notes, tasks or diff.";

/// Parameters for `workflow_get_turns`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GetTurnsParams {
    /// Sessions (`01`), turns (`01.02`) or ranges (`01.01..01.03`) to list (default: all turns).
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    pub implicit_ids: Option<Vec<String>>,
    #[serde(flatten)]
    pub page: PageParams,
}

/// Touched paths listed per turn.
const PATHS_SHOWN: usize = 8;

/// One entry per turn: title, counts and touched paths. Without `entries` the most recent
/// page is shown, because the latest turns matter most.
pub fn get_turns(project: &Project, cfg: &ServerConfig, p: GetTurnsParams) -> Result<Value> {
    let default_limit = cfg.default_limit;
    let flat = project.flat_keys();
    let indexes = project.turn_indexes(&flat, p.implicit_ids.as_deref())?;
    let total = indexes.len();
    let (offset, len) = if p.page.entries.is_some() {
        p.page.window_in(default_limit, total)?
    } else {
        let len = default_limit.max(1);
        (total.saturating_sub(len), len)
    };
    let turns: Vec<Value> = indexes
        .iter()
        .skip(offset)
        .take(len)
        .filter_map(|i| project.turn(flat[*i]))
        .map(|turn| turn_entry(project, turn))
        .collect();
    let shown = turns.len();
    let mut out = json!({ "turns": turns, "total": total });
    if shown > 0 && shown < total {
        out["showing"] = json!(format!("{}..{}", offset + 1, offset + shown));
    }
    Ok(out)
}

fn turn_entry(project: &Project, turn: &TurnRecord) -> Value {
    let mut m = turn.header();
    if turn.key == project.active {
        m.insert("active".into(), json!(true));
    }
    m.insert("notes".into(), json!(turn.notes.len()));
    m.insert("handover_tasks".into(), json!(turn.tasks.len()));
    m.insert("updates".into(), json!(turn.updates.len()));
    let paths = project.touched_paths(turn.key);
    if !paths.is_empty() {
        let shown: Vec<&String> = paths.iter().take(PATHS_SHOWN).collect();
        m.insert("files".into(), json!(shown));
        if paths.len() > PATHS_SHOWN {
            m.insert("files_omitted".into(), json!(paths.len() - PATHS_SHOWN));
        }
    }
    Value::Object(m)
}
