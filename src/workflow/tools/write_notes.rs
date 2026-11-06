//! `workflow_write_notes`: notes of the active turn. The operations are `engine::notes`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::common::Result;
use crate::common::params::lenient_vec;
use crate::workflow::Project;
use crate::workflow::engine::items::NoteWriteOp;
use crate::workflow::engine::session::model::TurnRecord;

pub const DESCRIPTION: &str = "Edit the active turn's notes with `ops` (`Append`, `Edit`, `Delete`). Keep only durable facts or decisions that a later turn cannot guess, with their reason or implication. Omit progress, build status, duplicates and what the file updates already show; leave notes empty when nothing qualifies. Link related notes with `ref_notes`. IDs in one call use the numbering from before the call, so several deletes never shift each other. Optional `title` names the active turn when it has none; a titled turn keeps its title. Returns the remaining note IDs.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteNotesParams {
    #[serde(deserialize_with = "lenient_vec")]
    #[schemars(with = "Vec<NoteWriteOp>")]
    pub ops: Vec<NoteWriteOp>,
    /// Title for the active turn if it has none (single line). A titled turn keeps its title.
    #[serde(default)]
    pub title: Option<String>,
}

pub fn write_notes(project: &mut Project, p: WriteNotesParams) -> Result<Value> {
    let mut turn = project.active_turn()?.clone();
    project.apply_note_ops(&mut turn, &p.ops)?;
    let title = turn.adopt_title(p.title.as_deref());
    let renumbered = p
        .ops
        .iter()
        .any(|op| matches!(op, NoteWriteOp::Delete { .. }));
    let mut out = notes_json(&turn, renumbered);
    title.report(&turn, &mut out);
    project.replace_active(turn)?;
    Ok(out)
}

fn notes_json(turn: &TurnRecord, renumbered: bool) -> Value {
    let mut out = json!({
        "turn_id": turn.key.to_string(),
        "note_ids": turn.note_ids(),
    });
    if let Some(title) = &turn.title {
        out["title"] = json!(title);
    }
    if renumbered {
        out["renumbered"] = json!("note_ids lists the notes that remain, numbered from 01");
    }
    out
}
