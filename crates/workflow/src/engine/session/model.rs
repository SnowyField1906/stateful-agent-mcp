//! In-memory representation of sessions, turns, notes and handover tasks, and the note and task
//! DTOs returned to the agent.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::engine::items::clean_title;
use crate::engine::ops::FileOp;

/// Position of a turn: `(session, turn)`, displayed as `SS.TT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TurnKey {
    pub session: u32,
    pub turn: u32,
}

impl std::str::FromStr for TurnKey {
    type Err = &'static str;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::parse(s).ok_or("expected SS.TT")
    }
}

impl TurnKey {
    pub const fn new(session: u32, turn: u32) -> Self {
        Self { session, turn }
    }

    /// Parse exactly `SS.TT`.
    pub fn parse(s: &str) -> Option<Self> {
        let (session, turn) = s.trim().split_once('.')?;
        Some(Self::new(session.parse().ok()?, turn.parse().ok()?))
    }
}

impl fmt::Display for TurnKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}.{:02}", self.session, self.turn)
    }
}

/// Typed identifier for a note: `{session}.{turn}.note.{index}`, e.g. `01.02.note.01`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NoteId {
    pub key: TurnKey,
    pub index: usize,
}

impl NoteId {
    pub const fn new(key: TurnKey, index: usize) -> Self {
        Self { key, index }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().splitn(4, '.');
        let (a, b, kind, rest) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if kind != "note" {
            return None;
        }
        let key = TurnKey::new(a.parse().ok()?, b.parse().ok()?);
        let num: usize = rest.parse().ok()?;
        Some(Self::new(key, num.checked_sub(1)?))
    }
}

impl fmt::Display for NoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.note.{:02}", self.key, self.index + 1)
    }
}

/// Typed identifier for a handover task: `{session}.{turn}.task.{index}`, e.g. `01.02.task.01`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId {
    pub key: TurnKey,
    pub index: usize,
}

impl TaskId {
    pub const fn new(key: TurnKey, index: usize) -> Self {
        Self { key, index }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().splitn(4, '.');
        let (a, b, kind, rest) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if kind != "task" {
            return None;
        }
        let key = TurnKey::new(a.parse().ok()?, b.parse().ok()?);
        let num: usize = rest.parse().ok()?;
        Some(Self::new(key, num.checked_sub(1)?))
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.task.{:02}", self.key, self.index + 1)
    }
}

/// Typed identifier for a turn file update: `{session}.{turn}.update.{path}`, e.g. `01.02.update.src/lib.rs`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UpdateId {
    pub key: TurnKey,
    pub path: String,
}

impl UpdateId {
    pub fn new(key: TurnKey, path: impl Into<String>) -> Self {
        Self {
            key,
            path: path.into(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().splitn(4, '.');
        let (a, b, kind, rest) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if kind != "update" || rest.is_empty() {
            return None;
        }
        let key = TurnKey::new(a.parse().ok()?, b.parse().ok()?);
        Some(Self::new(key, rest))
    }
}

impl fmt::Display for UpdateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.update.{}", self.key, self.path)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteRecord {
    pub content: String,
    pub ref_notes: Vec<String>,
}

/// Short-term work handed over to the next turn or session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoverTaskRecord {
    pub content: String,
    pub user_explicit: bool,
    /// Origin ID when this task was rolled over from an earlier turn.
    pub rolled_over_from: Option<String>,
}

/// A note as returned to the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NoteDto {
    /// `{session}.{turn}.note.{index}`, for example `01.02.note.01`.
    pub id: String,
    pub content: String,
    /// Referenced notes, expanded recursively.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ref_notes: Vec<NoteDto>,
    /// IDs of referenced notes that the handover leaves out below its depth limit; `get_notes`
    /// returns them with the whole chain.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_expanded: Vec<String>,
}

/// A handover task as returned to the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HandoverTaskDto {
    /// `{session}.{turn}.task.{index}`, for example `01.02.task.01`.
    pub id: String,
    pub content: String,
    /// The user asked for this task explicitly.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub user_explicit: bool,
    /// ID of the task this one was rolled over from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rolled_over_from: Option<String>,
}

/// What the `title` argument of a `write_*` call did to the active turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleChange {
    /// No title was given, it was blank, or it equals the turn's title.
    Unchanged,
    /// The untitled turn took the title.
    Named,
    /// The turn already has a different title. Titles cannot change, so the argument was ignored.
    Kept,
}

impl TitleChange {
    /// Add the outcome to a tool's output: the title when this call set it, and a note when the
    /// argument was ignored.
    pub fn report(self, turn: &TurnRecord, out: &mut Value) {
        let Some(title) = &turn.title else { return };
        match self {
            Self::Unchanged => {}
            Self::Named => out["title"] = json!(title),
            Self::Kept => {
                out["title"] = json!(title);
                out["note"] = json!(
                    "the turn already has this title and it cannot be changed, so `title` was ignored"
                );
            }
        }
    }
}

/// Title of the turn that records edits made to tracked files outside the ledger.
pub const EXTERNAL_EDIT_TITLE: &str = "[External disk edit]";

#[derive(Clone, Debug)]
pub struct TurnRecord {
    pub key: TurnKey,
    pub title: Option<String>,
    pub notes: Vec<NoteRecord>,
    /// Net file changes of this turn (never contains `Recover`).
    pub updates: Vec<FileOp>,
    pub tasks: Vec<HandoverTaskRecord>,
}

impl TurnRecord {
    pub fn new(key: TurnKey) -> Self {
        Self {
            key,
            title: None,
            notes: Vec::new(),
            updates: Vec::new(),
            tasks: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.notes.is_empty()
            && self.updates.is_empty()
            && self.tasks.is_empty()
    }

    /// True when the turn has no title, notes or tasks: it is empty or holds only file edits, so
    /// no item of it can be referenced by ID.
    pub fn is_file_edits_only(&self) -> bool {
        self.title.is_none() && self.notes.is_empty() && self.tasks.is_empty()
    }

    /// True for a turn the ledger opened by itself to record edits made to tracked files outside
    /// it. It holds no notes or tasks, so a handover is never read from it.
    pub fn is_external_edit(&self) -> bool {
        self.title.as_deref() == Some(EXTERNAL_EDIT_TITLE)
            && self.notes.is_empty()
            && self.tasks.is_empty()
    }

    /// Apply the `title` argument of a `write_*` call. An untitled turn takes it as a single line;
    /// a titled turn keeps its title, because titles cannot be changed later.
    pub fn adopt_title(&mut self, raw: Option<&str>) -> TitleChange {
        let Some(title) = raw.and_then(clean_title) else {
            return TitleChange::Unchanged;
        };
        match &self.title {
            None => {
                self.title = Some(title);
                TitleChange::Named
            }
            Some(current) if *current == title => TitleChange::Unchanged,
            Some(_) => TitleChange::Kept,
        }
    }

    /// `{turn_id, title?}`: the object that a turn's item lists are attached to.
    pub fn header(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("turn_id".into(), json!(self.key.to_string()));
        if let Some(title) = &self.title {
            m.insert("title".into(), json!(title));
        }
        m
    }

    pub fn note_id(key: TurnKey, idx: usize) -> String {
        NoteId::new(key, idx).to_string()
    }

    pub fn note_ids(&self) -> Vec<String> {
        (0..self.notes.len())
            .map(|i| Self::note_id(self.key, i))
            .collect()
    }

    pub fn task_dtos(&self) -> Vec<HandoverTaskDto> {
        self.tasks
            .iter()
            .enumerate()
            .map(|(i, t)| HandoverTaskDto {
                id: TaskId::new(self.key, i).to_string(),
                content: t.content.clone(),
                user_explicit: t.user_explicit,
                rolled_over_from: t.rolled_over_from.clone(),
            })
            .collect()
    }
}
