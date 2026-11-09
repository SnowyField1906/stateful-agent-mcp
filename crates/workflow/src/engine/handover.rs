//! What opening a session reports to the agent: the usage guide, the previous turn with the notes
//! and handover tasks it passed on, and a summary of the sessions. `workflow_register` wraps these
//! in its response.

use serde_json::{Map, Value, json};

use super::Project;
use super::instructions as text;
use super::ops::summary::op_summary;
use super::project::seed_guide;
use super::session::model::{HandoverTaskDto, TurnKey, TurnRecord};
use crate::common::{AgentError, AgentUseMode, Result};

/// Sessions listed in the registration response (most recent).
const SESSIONS_SHOWN: usize = 10;

/// Turns searched backwards for notes when the last turn recorded none.
const NOTE_FALLBACK_TURNS: usize = 3;

/// The usage guide for the configured mode and, if the project has one, its own
/// `INSTRUCTION.md`.
pub fn instructions(project: &Project, mode: AgentUseMode) -> Result<Value> {
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

/// Said in the handover when a note leaves referenced notes out below the depth limit.
const NOTES_CUT_HINT: &str = "Notes show their references one level deep; `not_expanded` lists the IDs left out. `workflow_get_notes` with a note ID (or the turn) returns the whole chain.";

/// Whether any note of `notes`, or of the notes it references, lists references left out.
fn has_cut(notes: &[Value]) -> bool {
    notes.iter().any(|note| {
        note.get("not_expanded").is_some()
            || note
                .get("ref_notes")
                .and_then(Value::as_array)
                .is_some_and(|inner| has_cut(inner))
    })
}

/// The latest non-empty turn before the active one, as shown to the agent. When it recorded no
/// handover tasks or notes, those of the turns before it are shown with the turn they come from.
pub fn last_turn(project: &Project) -> Option<Value> {
    let turn = last_closed_turn(project)?;
    let mut last = turn_json(project, turn);
    if turn.tasks.is_empty()
        && let Some((from_key, tasks)) = latest_tasks_before(project, turn.key)
        && let Some(obj) = last.as_object_mut()
    {
        obj.insert("handover_tasks".into(), json!(tasks));
        obj.insert(
            "handover_tasks_from_turn".into(),
            json!(from_key.to_string()),
        );
    }
    if turn.notes.is_empty()
        && let Some(earlier) = latest_notes_before(project, turn.key)
        && let Some(obj) = last.as_object_mut()
    {
        obj.insert("notes".into(), json!(project.handover_note_dtos(earlier)));
        obj.insert("notes_from_turn".into(), json!(earlier.key.to_string()));
    }
    let cut = last
        .get("notes")
        .and_then(Value::as_array)
        .is_some_and(|notes| has_cut(notes));
    if cut && let Some(obj) = last.as_object_mut() {
        obj.insert("notes_hint".into(), json!(NOTES_CUT_HINT));
    }
    Some(last)
}

/// One line per session, the most recent [`SESSIONS_SHOWN`] of them, and the number of sessions.
pub fn recent_sessions(project: &Project) -> (Vec<String>, usize) {
    let summaries: Vec<String> = project
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
        .collect();
    let total = summaries.len();
    let skip = total.saturating_sub(SESSIONS_SHOWN);
    (summaries[skip..].to_vec(), total)
}

/// The latest non-empty turn before the active one.
fn last_closed_turn(project: &Project) -> Option<&TurnRecord> {
    project
        .flat_keys()
        .into_iter()
        .rev()
        .filter(|k| *k != project.active)
        .filter_map(|k| project.turn(k))
        .find(|t| !t.is_empty() && !t.is_external_edit())
}

/// The latest turn with notes among the `NOTE_FALLBACK_TURNS` turns before `before_key`, so a
/// small follow-up turn does not hide the decisions an earlier turn recorded.
fn latest_notes_before(project: &Project, before_key: TurnKey) -> Option<&TurnRecord> {
    project
        .flat_keys()
        .into_iter()
        .rev()
        .filter(|k| *k < before_key)
        .filter_map(|k| project.turn(k))
        .filter(|t| !t.is_external_edit())
        .take(NOTE_FALLBACK_TURNS)
        .find(|t| !t.notes.is_empty())
}

/// The most recent non-empty handover task list from turns strictly before `before_key`.
fn latest_tasks_before(
    project: &Project,
    before_key: TurnKey,
) -> Option<(TurnKey, Vec<HandoverTaskDto>)> {
    let candidate = project
        .flat_keys()
        .into_iter()
        .rev()
        .filter(|k| *k < before_key)
        .filter_map(|k| project.turn(k))
        .find(|t| !t.is_external_edit())?;
    if !candidate.tasks.is_empty() {
        Some((candidate.key, candidate.task_dtos()))
    } else {
        None
    }
}

/// A turn as shown to the agent. Updates are summaries; `get_files_diff` returns the content.
fn turn_json(project: &Project, turn: &TurnRecord) -> Value {
    let mut m = turn.header();
    if !turn.notes.is_empty() {
        m.insert("notes".into(), json!(project.handover_note_dtos(turn)));
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
