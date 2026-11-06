//! `workflow_get_notes`.

use serde_json::{Value, json};

use crate::common::Result;
use crate::workflow::Project;
use crate::workflow::engine::items::GetItemsParams;
use crate::workflow::engine::selector::Sel;

pub const DESCRIPTION: &str = "Read notes of past turns (`ref_notes` are expanded recursively). `implicit_ids` takes selectors: session `01`, turn `01.02`, range `01.01..01.03` (`01.02..` ends at the active turn), `HEAD`, `HEAD~1`, or a note ID `01.02.note.01`.";

pub fn get_notes(project: &Project, p: GetItemsParams) -> Result<Value> {
    project.items_by_selectors(
        &p.implicit_ids,
        "notes",
        |sel| match sel {
            Sel::Note(key, idx) => Ok(Some((key, idx))),
            Sel::Turns { .. } => Ok(None),
            _ => Err("a note ID, turn, session or range"),
        },
        |turn| json!(project.note_dtos(turn)),
        |turn, idx| project.note_dto(turn, idx).map(|n| json!(n)),
    )
}
