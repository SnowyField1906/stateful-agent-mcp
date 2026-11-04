//! Notes of the active turn.

use serde_json::{Value, json};

use crate::error::{AgentError, Result};
use crate::params::NoteWriteOp;
use crate::session::model::{NoteRecord, TurnKey, TurnRecord};

use super::items::{clean_text, item_index};
use super::project::Project;
use super::selector::{Sel, parse_item};

impl Project {
    pub fn write_notes(&mut self, ops: &[NoteWriteOp]) -> Result<Value> {
        let mut turn = self.active_turn()?.clone();
        self.apply_note_ops(&mut turn, ops)?;
        let renumbered = ops.iter().any(|op| matches!(op, NoteWriteOp::Delete { .. }));
        let out = notes_json(&turn, renumbered);
        self.replace_active(turn)?;
        Ok(out)
    }

    /// Note IDs in `ops` refer to the numbering at the start of the call, so deleting several
    /// notes in one call never shifts the ones that follow. Notes appended in the call continue
    /// that numbering. When the call is done the remaining notes are renumbered without gaps.
    pub(super) fn apply_note_ops(&self, turn: &mut TurnRecord, ops: &[NoteWriteOp]) -> Result<()> {
        let key = turn.key;
        let mut slots: Vec<Option<NoteRecord>> = turn.notes.iter().cloned().map(Some).collect();
        for op in ops {
            match op {
                NoteWriteOp::Append { note_content, ref_notes } => {
                    let refs = self.check_refs(key, &slots, ref_notes.as_deref())?;
                    slots.push(Some(NoteRecord {
                        content: clean_text(note_content, "note_content")?,
                        ref_notes: refs,
                    }));
                }
                NoteWriteOp::Edit { note_id, note_content, ref_notes } => {
                    let i = live_slot(&slots, note_id, key)?;
                    let refs = match ref_notes {
                        Some(r) => Some(self.check_refs(key, &slots, Some(r))?),
                        None => None,
                    };
                    let content = clean_text(note_content, "note_content")?;
                    if let Some(note) = slots[i].as_mut() {
                        note.content = content;
                        if let Some(refs) = refs {
                            note.ref_notes = refs;
                        }
                    }
                }
                NoteWriteOp::Delete { note_id } => {
                    let i = live_slot(&slots, note_id, key)?;
                    slots[i] = None;
                }
            }
        }
        turn.notes = compact(key, slots);
        Ok(())
    }

    /// Validate note references and normalize them to `SS.TT.note.NN`.
    fn check_refs(
        &self,
        key: TurnKey,
        slots: &[Option<NoteRecord>],
        refs: Option<&[String]>,
    ) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for raw in refs.unwrap_or_default() {
            let Some(Sel::Note(k, idx)) = parse_item(raw.trim()) else {
                return Err(AgentError::invalid("ref_notes", format!("`{raw}` is not a note ID like 01.02.note.01")));
            };
            let exists = if k == key {
                slots.get(idx).is_some_and(Option::is_some)
            } else {
                self.turn(k).is_some_and(|t| idx < t.notes.len())
            };
            if !exists {
                return Err(AgentError::NotFound(format!("note {}", raw.trim())));
            }
            let id = format!("{k}.note.{:02}", idx + 1);
            if !out.contains(&id) {
                out.push(id);
            }
        }
        Ok(out)
    }
}

/// Index of a note that is still present, from `SS.TT.note.NN` or a bare number.
fn live_slot(slots: &[Option<NoteRecord>], raw: &str, key: TurnKey) -> Result<usize> {
    let i = item_index(raw, key, "note", slots.len())?;
    if slots[i].is_none() {
        return Err(AgentError::NotFound(format!("note {} was already deleted in this call", raw.trim())));
    }
    Ok(i)
}

fn notes_json(turn: &TurnRecord, renumbered: bool) -> Value {
    let mut out = json!({
        "turn_id": turn.key.to_string(),
        "note_ids": turn.note_dtos().into_iter().map(|n| n.id).collect::<Vec<_>>(),
    });
    if let Some(title) = &turn.title {
        out["title"] = json!(title);
    }
    if renumbered {
        out["renumbered"] = json!("note_ids lists the notes that remain, numbered from 01");
    }
    out
}

/// Drop deleted slots and renumber references to notes of the same turn to match.
fn compact(key: TurnKey, slots: Vec<Option<NoteRecord>>) -> Vec<NoteRecord> {
    let mut new_index: Vec<Option<usize>> = Vec::with_capacity(slots.len());
    let mut next = 0;
    for slot in &slots {
        if slot.is_some() {
            new_index.push(Some(next));
            next += 1;
        } else {
            new_index.push(None);
        }
    }
    slots
        .into_iter()
        .flatten()
        .map(|note| {
            let ref_notes = note
                .ref_notes
                .iter()
                .filter_map(|r| match parse_item(r) {
                    Some(Sel::Note(k, i)) if k == key => new_index
                        .get(i)
                        .copied()
                        .flatten()
                        .map(|n| format!("{key}.note.{:02}", n + 1)),
                    _ => Some(r.clone()),
                })
                .collect();
            NoteRecord { content: note.content, ref_notes }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_renumbers_same_turn_refs_and_drops_deleted_ones() {
        let key = TurnKey::new(1, 1);
        let note = |content: &str, refs: &[&str]| {
            Some(NoteRecord {
                content: content.into(),
                ref_notes: refs.iter().map(|s| s.to_string()).collect(),
            })
        };
        let slots = vec![
            note("a", &[]),
            None,
            note("c", &[]),
            note("d", &["01.01.note.01", "01.01.note.02", "01.01.note.03", "02.01.note.05"]),
        ];
        let notes = compact(key, slots);
        assert_eq!(notes.len(), 3);
        assert_eq!(
            notes[2].ref_notes,
            vec!["01.01.note.01", "01.01.note.02", "02.01.note.05"]
        );
    }
}
