//! Applying note operations to a turn.

use std::collections::BTreeSet;

use crate::common::{AgentError, Result};

use super::Project;
use super::items::{NoteWriteOp, clean_text, live_slot};
use super::selector::{Sel, parse_item};
use super::session::model::{NoteDto, NoteRecord, TurnKey, TurnRecord};

impl Project {
    /// Note IDs in `ops` refer to the numbering at the start of the call, so deleting several
    /// notes in one call never shifts the ones that follow. Notes appended in the call continue
    /// that numbering. When the call is done the remaining notes are renumbered without gaps.
    pub(crate) fn apply_note_ops(&self, turn: &mut TurnRecord, ops: &[NoteWriteOp]) -> Result<()> {
        let key = turn.key;
        let mut slots: Vec<Option<NoteRecord>> = turn.notes.iter().cloned().map(Some).collect();
        for op in ops {
            match op {
                NoteWriteOp::Append {
                    note_content,
                    ref_notes,
                } => {
                    let refs = self.check_refs(key, &slots, ref_notes.as_deref())?;
                    slots.push(Some(NoteRecord {
                        content: clean_text(note_content, "note_content")?,
                        ref_notes: refs,
                    }));
                }
                NoteWriteOp::Edit {
                    note_id,
                    note_content,
                    ref_notes,
                } => {
                    let i = live_slot(&slots, note_id, key, "note")?;
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
                    let i = live_slot(&slots, note_id, key, "note")?;
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
                return Err(AgentError::invalid(
                    "ref_notes",
                    format!("`{raw}` is not a note ID like 01.02.note.01"),
                ));
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

    /// All notes of `turn` as DTOs, with `ref_notes` expanded recursively.
    pub(crate) fn note_dtos(&self, turn: &TurnRecord) -> Vec<NoteDto> {
        (0..turn.notes.len())
            .filter_map(|idx| self.note_dto(turn, idx))
            .collect()
    }

    /// One note of `turn` as a DTO, with `ref_notes` expanded recursively.
    pub(crate) fn note_dto(&self, turn: &TurnRecord, idx: usize) -> Option<NoteDto> {
        let mut seen = BTreeSet::new();
        self.resolve_note_dto(turn, idx, &mut seen)
    }

    fn resolve_note_dto(
        &self,
        turn: &TurnRecord,
        idx: usize,
        seen: &mut BTreeSet<String>,
    ) -> Option<NoteDto> {
        let note = turn.notes.get(idx)?;
        let id = TurnRecord::note_id(turn.key, idx);
        seen.insert(id.clone());
        let ref_notes = note
            .ref_notes
            .iter()
            .filter_map(|raw| {
                let Sel::Note(k, i) = parse_item(raw)? else {
                    return None;
                };
                let ref_id = TurnRecord::note_id(k, i);
                if seen.contains(&ref_id) {
                    return None;
                }
                let target = self.turn(k)?;
                self.resolve_note_dto(target, i, seen)
            })
            .collect();
        Some(NoteDto {
            id,
            content: note.content.clone(),
            ref_notes,
        })
    }
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
            NoteRecord {
                content: note.content,
                ref_notes,
            }
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
            note(
                "d",
                &[
                    "01.01.note.01",
                    "01.01.note.02",
                    "01.01.note.03",
                    "02.01.note.05",
                ],
            ),
        ];
        let notes = compact(key, slots);
        assert_eq!(notes.len(), 3);
        assert_eq!(
            notes[2].ref_notes,
            vec!["01.01.note.01", "01.01.note.02", "02.01.note.05"]
        );
    }
}
