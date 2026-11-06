//! `workflow_register`: the response that opens a session.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::common::{AgentError, AgentUseMode, Result, ServerConfig};
use crate::workflow::Project;
use crate::workflow::engine::instructions as text;
use crate::workflow::engine::ops::summary::op_summary;
use crate::workflow::engine::project::seed_guide;
use crate::workflow::engine::session::model::{HandoverTaskDto, TurnKey, TurnRecord};

pub const DESCRIPTION: &str = "Open a project (`folder_path`, absolute) and start or resume its session. Call this first. Returns the active turn ID, the usage guide, the project's own instructions, `language_capabilities` (the actions `langtool_run_action` accepts) and the previous turn's notes, handover tasks and file updates.";

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

/// Sessions listed in the registration response (most recent).
const SESSIONS_SHOWN: usize = 10;

pub fn register(cfg: &ServerConfig, p: RegisterParams) -> Result<(Project, Value)> {
    let project = Project::open(&p.folder_path)?;
    let show_instructions = p.show_instructions.unwrap_or(true);
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
    if show_instructions {
        out.insert(
            "instructions".into(),
            json!(instructions_text(&project, cfg.use_mode)?),
        );
    }
    if show_last_turn && let Some(turn) = last_closed_turn(&project) {
        let mut last = turn_json(&project, turn);
        if turn.tasks.is_empty()
            && let Some((from_key, tasks)) = latest_tasks_before(&project, turn.key)
            && let Some(obj) = last.as_object_mut()
        {
            obj.insert("handover_tasks".into(), json!(tasks));
            obj.insert(
                "handover_tasks_from_turn".into(),
                json!(from_key.to_string()),
            );
        }
        if turn.notes.is_empty()
            && let Some(earlier) = latest_notes_before(&project, turn.key)
            && let Some(obj) = last.as_object_mut()
        {
            obj.insert("notes".into(), json!(project.note_dtos(earlier)));
            obj.insert("notes_from_turn".into(), json!(earlier.key.to_string()));
        }
        out.insert("last_turn".into(), last);
    }
    let summaries = session_summaries(&project);
    let skip = summaries.len().saturating_sub(SESSIONS_SHOWN);
    out.insert("sessions".into(), json!(&summaries[skip..]));
    if skip > 0 {
        out.insert("sessions_total".into(), json!(summaries.len()));
    }
    Ok((project, Value::Object(out)))
}

/// The latest non-empty turn before the active one.
fn last_closed_turn(project: &Project) -> Option<&TurnRecord> {
    project
        .flat_keys()
        .into_iter()
        .rev()
        .filter(|k| *k != project.active)
        .filter_map(|k| project.turn(k))
        .find(|t| !t.is_empty())
}

/// Turns searched backwards for notes when the last turn recorded none.
const NOTE_FALLBACK_TURNS: usize = 3;

/// The latest turn with notes among the `NOTE_FALLBACK_TURNS` turns before `before_key`, so a
/// small follow-up turn does not hide the decisions an earlier turn recorded.
fn latest_notes_before(project: &Project, before_key: TurnKey) -> Option<&TurnRecord> {
    project
        .flat_keys()
        .into_iter()
        .rev()
        .filter(|k| *k < before_key)
        .take(NOTE_FALLBACK_TURNS)
        .filter_map(|k| project.turn(k))
        .find(|t| !t.notes.is_empty())
}

/// The most recent non-empty handover task list from turns strictly before `before_key`.
fn latest_tasks_before(
    project: &Project,
    before_key: TurnKey,
) -> Option<(TurnKey, Vec<HandoverTaskDto>)> {
    project
        .flat_keys()
        .into_iter()
        .rev()
        .filter(|k| *k < before_key)
        .filter_map(|k| project.turn(k))
        .find(|t| !t.tasks.is_empty())
        .map(|t| (t.key, t.task_dtos()))
}

fn session_summaries(project: &Project) -> Vec<String> {
    project
        .sessions
        .iter()
        .map(|(n, turns)| {
            let notes: usize = turns.iter().map(|t| t.notes.len()).sum();
            let tasks: usize = turns.iter().map(|t| t.tasks.len()).sum();
            let updates: usize = turns.iter().map(|t| t.updates.len()).sum();
            format!(
                "{n:02}: {} turns, {notes} notes, {tasks} handover tasks, {updates} file updates",
                turns.len()
            )
        })
        .collect()
}

/// The usage guide for the configured mode and, if the project has one, its own
/// `INSTRUCTION.md`.
fn instructions_text(project: &Project, mode: AgentUseMode) -> Result<Value> {
    let guide = seed_guide(&project.agents_dir, mode)?;
    let mut out = Map::new();
    out.insert("guide".into(), json!(guide.trim_end()));
    let path = project.agents_dir.join(text::PROJECT_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => {
            out.insert("project".into(), json!(text.trim_end()));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(AgentError::io_at(&path, e)),
    }
    Ok(Value::Object(out))
}

/// A turn as shown to the agent. Updates are summaries; `get_files_diff` returns the content.
fn turn_json(project: &Project, turn: &TurnRecord) -> Value {
    let mut m = turn.header();
    if !turn.notes.is_empty() {
        m.insert("notes".into(), json!(project.note_dtos(turn)));
    }
    if !turn.updates.is_empty() {
        let updates: Vec<String> = turn.updates.iter().map(op_summary).collect();
        m.insert("updates".into(), json!(updates));
    }
    if !turn.tasks.is_empty() {
        m.insert("handover_tasks".into(), json!(turn.task_dtos()));
    }
    Value::Object(m)
}
