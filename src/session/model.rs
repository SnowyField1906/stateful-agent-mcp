//! In-memory representation of sessions, turns, notes and handover tasks.

use std::fmt;

use crate::params::{FileOp, HandoverTaskDto, NoteDto};

/// Position of a turn: `(session, turn)`, displayed as `SS.TT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TurnKey {
    pub session: u32,
    pub turn: u32,
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

    pub fn note_dtos(&self) -> Vec<NoteDto> {
        self.notes
            .iter()
            .enumerate()
            .map(|(i, n)| NoteDto {
                id: format!("{}.note.{:02}", self.key, i + 1),
                content: n.content.clone(),
                ref_notes: n.ref_notes.clone(),
            })
            .collect()
    }

    pub fn task_dtos(&self) -> Vec<HandoverTaskDto> {
        self.tasks
            .iter()
            .enumerate()
            .map(|(i, t)| HandoverTaskDto {
                id: format!("{}.task.{:02}", self.key, i + 1),
                content: t.content.clone(),
                user_explicit: t.user_explicit,
                rolled_over_from: t.rolled_over_from.clone(),
            })
            .collect()
    }
}
