//! Selector syntax shared by the query tools and by `Recover`.
//!
//! - `01` session, `01.02` turn, `HEAD` active turn, `HEAD~N` the turn N before it
//! - `A..B` inclusive range of the above; `A..` runs to `HEAD`, `..B` starts at the first turn.
//!   A range written backwards (`01.05..01.02`) is reversed.
//! - `01.02.note.03`, `01.02.task.01`, `01.02.update.src/a.rs` single items

use serde_json::{Value, json};

use crate::common::{AgentError, Result};

use super::project::Project;
use super::session::model::{NoteId, TaskId, TurnKey, TurnRecord, UpdateId};

#[derive(Debug)]
pub(crate) enum Sel {
    /// Inclusive indexes into the flat turn list.
    Turns {
        lo: usize,
        hi: usize,
        reversed: bool,
    },
    /// Zero-based item index within the turn.
    Note(TurnKey, usize),
    Task(TurnKey, usize),
    Update(TurnKey, String),
}

impl Project {
    pub(crate) fn parse_selector(&self, flat: &[TurnKey], raw: &str) -> Result<Sel> {
        let s = raw.trim();
        if s.is_empty() {
            return Err(AgentError::invalid("implicit_id", "empty selector"));
        }
        if let Some(item) = parse_item(s) {
            return Ok(item);
        }
        if let Some((from, to)) = s.split_once("..") {
            let lo = if from.trim().is_empty() {
                0
            } else {
                span(flat, from)?.0
            };
            let hi = if to.trim().is_empty() {
                flat.len() - 1
            } else {
                span(flat, to)?.1
            };
            return Ok(if lo <= hi {
                Sel::Turns {
                    lo,
                    hi,
                    reversed: false,
                }
            } else {
                Sel::Turns {
                    lo: hi,
                    hi: lo,
                    reversed: true,
                }
            });
        }
        let (lo, hi) = span(flat, s)?;
        Ok(Sel::Turns {
            lo,
            hi,
            reversed: false,
        })
    }

    /// Indexes into the flat turn list covered by `ids`; every turn when none are given.
    pub(crate) fn turn_indexes(
        &self,
        flat: &[TurnKey],
        ids: Option<&[String]>,
    ) -> Result<Vec<usize>> {
        let ids: Vec<&String> = ids
            .unwrap_or_default()
            .iter()
            .filter(|s| !s.trim().is_empty())
            .collect();
        if ids.is_empty() {
            return Ok((0..flat.len()).collect());
        }
        let mut out = Vec::new();
        for raw in ids {
            match self.parse_selector(flat, raw)? {
                Sel::Turns { lo, hi, .. } => out.extend(lo..=hi),
                _ => {
                    return Err(AgentError::invalid(
                        "implicit_ids",
                        format!("`{}` is not a session, turn or range", raw.trim()),
                    ));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// Shared by `get_notes` and `get_tasks`: `classify` maps a selector to an item reference,
    /// `None` for turn selectors, or the expected-kind message; `all` lists a turn's items; `one`
    /// picks one.
    pub(crate) fn items_by_selectors(
        &self,
        selectors: &[String],
        field: &str,
        classify: impl Fn(Sel) -> std::result::Result<Option<(TurnKey, usize)>, &'static str>,
        all: impl Fn(&TurnRecord) -> Value,
        one: impl Fn(&TurnRecord, usize) -> Option<Value>,
    ) -> Result<Value> {
        let flat = self.flat_keys();
        let mut groups: Vec<Value> = Vec::new();
        for raw in selectors {
            let sel = self.parse_selector(&flat, raw)?;
            let range = match &sel {
                Sel::Turns { lo, hi, .. } => Some((*lo, *hi)),
                _ => None,
            };
            match classify(sel).map_err(|expected| {
                AgentError::invalid(raw.as_str(), format!("expected {expected}"))
            })? {
                Some((key, idx)) => {
                    let turn = self
                        .turn(key)
                        .ok_or_else(|| AgentError::NotFound(format!("turn {key}")))?;
                    let item = one(turn, idx)
                        .ok_or_else(|| AgentError::NotFound(raw.trim().to_string()))?;
                    let mut group = turn.header();
                    group.insert(field.into(), json!([item]));
                    groups.push(Value::Object(group));
                }
                None => {
                    let (lo, hi) = range.unwrap_or((0, 0));
                    for key in &flat[lo..=hi] {
                        let Some(turn) = self.turn(*key) else {
                            continue;
                        };
                        let items = all(turn);
                        if items.as_array().is_some_and(|a| !a.is_empty()) {
                            let mut group = turn.header();
                            group.insert(field.into(), items);
                            groups.push(Value::Object(group));
                        }
                    }
                }
            }
        }
        Ok(Value::Array(groups))
    }
}

/// Parse `SS.TT.note.NN`, `SS.TT.task.NN` or `SS.TT.update.{path}`.
pub(crate) fn parse_item(s: &str) -> Option<Sel> {
    if let Some(id) = NoteId::parse(s) {
        return Some(Sel::Note(id.key, id.index));
    }
    if let Some(id) = TaskId::parse(s) {
        return Some(Sel::Task(id.key, id.index));
    }
    if let Some(id) = UpdateId::parse(s) {
        return Some(Sel::Update(id.key, id.path));
    }
    None
}

fn bad(raw: &str) -> AgentError {
    AgentError::invalid(
        "implicit_id",
        format!(
            "`{}` is not a session (01), turn (01.02), range (01.02..01.05), HEAD[~N] or item ID",
            raw.trim()
        ),
    )
}

/// First and last flat index covered by a session, turn or `HEAD` selector.
fn span(flat: &[TurnKey], raw: &str) -> Result<(usize, usize)> {
    let s = raw.trim();
    if let Some(rest) = s.strip_prefix("HEAD") {
        let back = match rest {
            "" => 0,
            "~" => 1,
            r => r
                .strip_prefix('~')
                .and_then(|n| n.parse::<usize>().ok())
                .ok_or_else(|| bad(raw))?,
        };
        let idx = (flat.len() - 1).checked_sub(back).ok_or_else(|| {
            AgentError::NotFound(format!("{s}: history has only {} turns", flat.len()))
        })?;
        return Ok((idx, idx));
    }
    if let Some(key) = TurnKey::parse(s) {
        let idx = flat
            .iter()
            .position(|k| *k == key)
            .ok_or_else(|| AgentError::NotFound(format!("turn {key}")))?;
        return Ok((idx, idx));
    }
    if let Ok(session) = s.parse::<u32>() {
        let first = flat.iter().position(|k| k.session == session);
        let last = flat.iter().rposition(|k| k.session == session);
        return match (first, last) {
            (Some(f), Some(l)) => Ok((f, l)),
            _ => Err(AgentError::NotFound(format!("session {session:02}"))),
        };
    }
    Err(bad(raw))
}
