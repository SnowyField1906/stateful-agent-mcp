//! `workflow_register`: the response that opens a session.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::Project;
use crate::common::{Result, ServerConfig};
use crate::engine::handover;

pub const DESCRIPTION: &str = "Open a project (`folder_path`, absolute) and start or resume its session. Call this first. Returns the active turn ID, the usage guide and the project's own instructions (left out on a repeat call for the same project when they have not changed; `show_instructions` forces them), `language_capabilities` (the actions `langtool_run_action` accepts) and the previous turn's notes, handover tasks and file updates. Files changed on disk outside the ledger are settled when a later batch changes them, not here.";

/// Said when the instructions are left out because this server returned them already.
const INSTRUCTIONS_HINT: &str = "Instructions are unchanged since this server last returned them for this project; call again with `show_instructions: true` to read them.";

/// The hash of the instructions last returned for each project root.
static SHOWN: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();

/// Said once the session opens with handover tasks left: where to find them in the code.
const OPEN_TASKS_HINT: &str = "Handover tasks are open: `langtool_summarize_codebase` resolves them to code locations in its `front:` row.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegisterParams {
    /// Absolute path of the project root.
    pub folder_path: String,
    /// Include the instruction files in the response. Default: on the first call for this project
    /// in this server, and again when they changed; off on a repeat call.
    #[serde(default)]
    pub show_instructions: Option<bool>,
    /// Include the last turn's notes, handover tasks and update summary (default true).
    #[serde(default)]
    pub show_last_turn_info: Option<bool>,
}

pub fn register(cfg: &ServerConfig, p: RegisterParams) -> Result<(Project, Value)> {
    let project = Project::open(&p.folder_path)?;
    let show_last_turn = p.show_last_turn_info.unwrap_or(true);

    let mut out = Map::new();
    out.insert("project_root".into(), json!(project.root.to_string_lossy()));
    out.insert("active_turn_id".into(), json!(project.active.to_string()));
    match project.language_config() {
        Ok(lang) => {
            if let Some(caps) = lang.capabilities_json() {
                out.insert("language_capabilities".into(), caps);
            }
        }
        Err(e) => {
            out.insert("language_config_error".into(), json!(e.to_string()));
        }
    }
    if p.show_instructions != Some(false) {
        let text = handover::instructions(&project, cfg.use_mode)?;
        let fresh = first_sight(&project.root.to_string_lossy(), &text);
        if fresh || p.show_instructions == Some(true) {
            out.insert("instructions".into(), text);
        } else {
            out.insert("instructions_hint".into(), json!(INSTRUCTIONS_HINT));
        }
    }
    let last = handover::last_turn(&project);
    let open_tasks = last
        .as_ref()
        .is_some_and(|last| last.get("handover_tasks").is_some());
    if show_last_turn && let Some(last) = last {
        out.insert("last_turn".into(), last);
    }
    if open_tasks {
        out.insert("hint".into(), json!(OPEN_TASKS_HINT));
    }
    let (shown, total) = handover::recent_sessions(&project);
    if total > shown.len() {
        out.insert("sessions_total".into(), json!(total));
    }
    out.insert("sessions".into(), json!(shown));
    Ok((project, Value::Object(out)))
}

/// Whether `instructions` differ from what this server last returned for `root`; records them.
fn first_sight(root: &str, instructions: &Value) -> bool {
    let mut hasher = DefaultHasher::new();
    instructions.to_string().hash(&mut hasher);
    let hash = hasher.finish();
    let mut shown = SHOWN
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    shown.insert(root.to_string(), hash) != Some(hash)
}
