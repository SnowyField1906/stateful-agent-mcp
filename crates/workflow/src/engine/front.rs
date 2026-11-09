//! Where the latest sessions stopped, as the code tools read it through `Workspace::work_front`.

use std::collections::BTreeSet;

use crate::common::{Front, ItemKind, Moved, OpenItem};

use super::Project;
use super::session::model::{NoteId, TaskId, TurnKey, TurnRecord};

/// Sessions that make up the latest ones: the last two that recorded something.
const SESSIONS: usize = 2;
/// Most items of `Front::open` and files of `Front::moved`.
const MOST_ITEMS: usize = 48;

impl Project {
    /// One pass over the turns of the latest sessions, newest first. A session is counted when it
    /// has a turn that recorded something.
    pub(crate) fn build_front(&self) -> Front {
        let flat = self.flat_keys();
        let recorded: Vec<&TurnRecord> = flat
            .iter()
            .filter_map(|key| self.turn(*key))
            .filter(|turn| !turn.is_empty())
            .collect();
        let sessions: BTreeSet<u32> = recorded.iter().map(|turn| turn.key.session).collect();
        let Some(first) = sessions.iter().rev().take(SESSIONS).last().copied() else {
            return Front::default();
        };

        let mut front = Front::default();
        let mut seen_paths = BTreeSet::new();
        // The tasks of the newest turn that has any are the list that stands.
        let mut standing: Option<TurnKey> = None;
        for (age, turn) in recorded.iter().rev().enumerate() {
            if turn.key.session < first {
                break;
            }
            if standing.is_none() && !turn.tasks.is_empty() {
                standing = Some(turn.key);
                for (index, task) in turn.tasks.iter().enumerate() {
                    front.open.push(OpenItem {
                        id: TaskId::new(turn.key, index).to_string(),
                        kind: ItemKind::Task,
                        text: task.content.clone(),
                    });
                }
            }
            for (index, note) in turn.notes.iter().enumerate() {
                front.open.push(OpenItem {
                    id: NoteId::new(turn.key, index).to_string(),
                    kind: ItemKind::Note,
                    text: note.content.clone(),
                });
            }
            for path in self.touched_paths(turn.key) {
                if front.moved.len() < MOST_ITEMS && seen_paths.insert(path.clone()) {
                    front.moved.push(Moved {
                        path,
                        turn: turn.key.to_string(),
                        age,
                        outside: turn.is_external_edit(),
                    });
                }
            }
        }
        front.open.truncate(MOST_ITEMS);
        front
    }
}
