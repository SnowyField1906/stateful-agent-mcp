//! `workflow_register`: the response that opens a session.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::Project;
use crate::common::{Result, ServerConfig};
use crate::engine::handover;

pub const DESCRIPTION: &str = "Open a project (`folder_path`, absolute) and start or resume its session. Call this first. Returns the active turn ID, the usage guide, the project's own instructions, `language_capabilities` (the actions `langtool_run_action` accepts) and the previous turn's notes, handover tasks and file updates. Files that changed on disk outside the ledger are recorded in a turn of their own and listed under `external_edits`.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegisterParams {
    /// Absolute path of the project root.
    pub folder_path: String,
    /// Include the instruction files in the response (default true).
    #[serde(default)]
    pub show_instructions: Option<bool>,
    /// Include the last turn's notes, handover tasks and update summary (default true).
    #[serde(default)]
    pub show_last_turn_info: Option<bool>,
}

pub fn register(cfg: &ServerConfig, p: RegisterParams) -> Result<(Project, Value)> {
    let mut project = Project::open(&p.folder_path)?;
    let external = project.sync_external_edits()?;
    let show_instructions = p.show_instructions.unwrap_or(true);
    let show_last_turn = p.show_last_turn_info.unwrap_or(true);

    let mut out = Map::new();
    out.insert("project_root".into(), json!(project.root.to_string_lossy()));
    out.insert("active_turn_id".into(), json!(project.active.to_string()));
    if let Some(external) = &external {
        out.insert("external_edits".into(), external.report());
    }
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
    if show_instructions {
        out.insert(
            "instructions".into(),
            handover::instructions(&project, cfg.use_mode)?,
        );
    }
    if show_last_turn && let Some(last) = handover::last_turn(&project) {
        out.insert("last_turn".into(), last);
    }
    let (shown, total) = handover::recent_sessions(&project);
    if total > shown.len() {
        out.insert("sessions_total".into(), json!(total));
    }
    out.insert("sessions".into(), json!(shown));
    Ok((project, Value::Object(out)))
}
