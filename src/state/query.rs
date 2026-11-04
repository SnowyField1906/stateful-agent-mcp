//! Read-only queries over recorded history: notes, handover tasks, diffs and file history.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::error::{AgentError, Result};
use crate::files::Tree;
use crate::files::net_ops::net_ops_with_moves;
use crate::files::path::{matches_pathspec, normalize_rel};
use crate::files::summary::{op_diff, op_summary};
use crate::params::FileOp;
use crate::session::model::{TurnKey, TurnRecord};

use super::project::Project;
use super::selector::Sel;

/// `{turn_id, title?}` object that item lists are attached to.
fn header(turn: &TurnRecord) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("turn_id".into(), json!(turn.key.to_string()));
    if let Some(title) = &turn.title {
        m.insert("title".into(), json!(title));
    }
    m
}

impl Project {
    // ----- notes and handover tasks --------------------------------------------------------

    pub fn notes_by_selectors(&self, selectors: &[String]) -> Result<Value> {
        self.items_by_selectors(
            selectors,
            "notes",
            |sel| match sel {
                Sel::Note(key, idx) => Ok(Some((key, idx))),
                Sel::Turns { .. } => Ok(None),
                _ => Err("a note ID, turn, session or range"),
            },
            |turn| json!(turn.note_dtos()),
            |turn, idx| turn.note_dtos().into_iter().nth(idx).map(|n| json!(n)),
        )
    }

    pub fn tasks_by_selectors(&self, selectors: &[String]) -> Result<Value> {
        self.items_by_selectors(
            selectors,
            "handover_tasks",
            |sel| match sel {
                Sel::Task(key, idx) => Ok(Some((key, idx))),
                Sel::Turns { .. } => Ok(None),
                _ => Err("a task ID, turn, session or range"),
            },
            |turn| json!(turn.task_dtos()),
            |turn, idx| turn.task_dtos().into_iter().nth(idx).map(|t| json!(t)),
        )
    }

    /// Shared implementation: `classify` maps a selector to an item reference, `None` for turn
    /// selectors, or the expected-kind message; `all` lists a turn's items; `one` picks one.
    fn items_by_selectors(
        &self,
        selectors: &[String],
        field: &str,
        classify: impl Fn(
            Sel,
        ) -> std::result::Result<
            Option<(crate::session::model::TurnKey, usize)>,
            &'static str,
        >,
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
                    let mut group = header(turn);
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
                            let mut group = header(turn);
                            group.insert(field.into(), items);
                            groups.push(Value::Object(group));
                        }
                    }
                }
            }
        }
        Ok(Value::Array(groups))
    }

    // ----- diff ---------------------------------------------------------------------------

    /// Net file changes of the selected turns as a unified diff, one block per operation.
    pub fn files_diff(&self, id: &str, paths: Option<&[String]>) -> Result<String> {
        let flat = self.flat_keys();
        let specs = self.pathspecs(paths)?;
        let ops: Vec<FileOp> = match self.parse_selector(&flat, id)? {
            Sel::Update(key, path) => {
                let turn = self
                    .turn(key)
                    .ok_or_else(|| AgentError::NotFound(format!("turn {key}")))?;
                let op = turn
                    .updates
                    .iter()
                    .find(|op| op.touches_path(&path))
                    .ok_or_else(|| AgentError::NotFound(id.trim().to_string()))?;
                vec![op.clone()]
            }
            Sel::Turns { lo, hi, reversed } if lo == hi && !reversed => {
                let turn = self
                    .turn(flat[lo])
                    .ok_or_else(|| AgentError::NotFound(format!("turn {}", flat[lo])))?;
                turn.updates
                    .iter()
                    .filter(|op| in_specs(op, &specs))
                    .cloned()
                    .collect()
            }
            Sel::Turns { lo, hi, reversed } => {
                let mut touched = BTreeSet::new();
                for key in &flat[lo..=hi] {
                    touched.extend(self.touched_paths(*key));
                }
                let moves = self.range_moves(&flat[lo..=hi], reversed);
                let (mut before, mut after) = (Tree::new(), Tree::new());
                for path in touched {
                    if !specs.is_empty() && !specs.iter().any(|s| matches_pathspec(&path, s)) {
                        continue;
                    }
                    before.insert(path.clone(), self.content_before(&flat, lo, &path)?);
                    after.insert(path.clone(), self.content_before(&flat, hi + 1, &path)?);
                }
                if reversed {
                    net_ops_with_moves(&after, &before, &moves)
                } else {
                    net_ops_with_moves(&before, &after, &moves)
                }
            }
            _ => {
                return Err(AgentError::invalid(
                    "implicit_id",
                    "expected a turn, session, range or update ID",
                ));
            }
        };
        if ops.is_empty() {
            return Ok("No changes.\n".to_string());
        }
        Ok(ops.iter().map(op_diff).collect())
    }

    // ----- history ------------------------------------------------------------------------

    /// Which turns changed which files, newest first. Content is omitted; use `files_diff`.
    pub fn files_history(
        &self,
        id: Option<&str>,
        paths: Option<&[String]>,
        limit: usize,
    ) -> Result<Value> {
        let flat = self.flat_keys();
        let (lo, hi) = match id {
            Some(raw) => match self.parse_selector(&flat, raw)? {
                Sel::Turns { lo, hi, .. } => (lo, hi),
                _ => {
                    return Err(AgentError::invalid(
                        "implicit_id",
                        "expected a turn, session or range",
                    ));
                }
            },
            None => (0, flat.len() - 1),
        };
        let mut tracked = self.pathspecs(paths)?;
        let mut turns: Vec<Value> = Vec::new();
        for key in flat[lo..=hi].iter().rev() {
            let Some(turn) = self.turn(*key) else {
                continue;
            };
            let mut updates: Vec<String> = Vec::new();
            for op in &turn.updates {
                if !in_specs(op, &tracked) {
                    continue;
                }
                updates.push(op_summary(op));
                // Follow renames backwards: from now on the old path is of interest too.
                if let FileOp::Move { path, to_path } = op
                    && !tracked.is_empty()
                    && tracked.iter().any(|s| matches_pathspec(to_path, s))
                {
                    tracked.push(path.clone());
                }
            }
            if !updates.is_empty() {
                let mut entry = header(turn);
                entry.insert("updates".into(), json!(updates));
                turns.push(Value::Object(entry));
            }
        }
        let total = turns.len();
        turns.truncate(limit.max(1));
        let mut out = json!({ "turns": turns });
        if total > limit.max(1) {
            out["omitted"] = json!(total - limit.max(1));
        }
        Ok(out)
    }

    /// Net move origins (`to_path -> from_path`) across a slice of turns, inverted when `reversed`.
    fn range_moves(&self, keys: &[TurnKey], reversed: bool) -> BTreeMap<String, String> {
        let mut moves: BTreeMap<String, String> = BTreeMap::new();
        for key in keys {
            let Some(turn) = self.turn(*key) else {
                continue;
            };
            for op in &turn.updates {
                match op {
                    FileOp::Move { path, to_path } => {
                        let orig = moves.remove(path).unwrap_or_else(|| path.clone());
                        if orig != *to_path {
                            moves.insert(to_path.clone(), orig);
                        }
                    }
                    FileOp::Delete { path } => {
                        moves.remove(path);
                    }
                    _ => {}
                }
            }
        }
        if reversed {
            moves.into_iter().map(|(to, from)| (from, to)).collect()
        } else {
            moves
        }
    }

    /// Normalize path filters; `.` and empty entries mean "everything".
    fn pathspecs(&self, paths: Option<&[String]>) -> Result<Vec<String>> {
        paths
            .unwrap_or_default()
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty() && *p != ".")
            .map(|p| normalize_rel(p, &self.root))
            .collect()
    }
}

fn in_specs(op: &FileOp, specs: &[String]) -> bool {
    specs.is_empty()
        || op
            .paths()
            .iter()
            .any(|p| specs.iter().any(|s| matches_pathspec(p, s)))
}
