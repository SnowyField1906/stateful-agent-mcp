//! The registered project, as the tools below `workflow` see it.
//!
//! The read tools of `filesystem` and `langtool` need a few things from the project: its root, the
//! last turn that changed a file, a file as of a past turn, and where the work of the latest
//! sessions stopped. [`Workspace`] names them, so those scopes depend on this trait and not on
//! `workflow::Project`, which implements it.

use std::path::Path;

use super::Result;

pub trait Workspace {
    /// The project's root directory.
    fn root(&self) -> &Path;

    /// The most recent turn that changed `rel` (a `/`-separated path below the root), as `SS.TT`.
    fn last_turn_touching(&self, rel: &str) -> Option<String>;

    /// Content of `rel` at the end of the turn(s) selected by `id` (a turn, session, `HEAD~N` or
    /// update ID); `None` when the file did not exist then.
    fn read_at(&self, id: &str, rel: &str) -> Result<Option<String>>;

    /// Content of `rel` just before the turn `id` (a turn or an update ID) ran; `None` when the file
    /// did not exist then, and for a workspace without a ledger.
    fn read_before(&self, _id: &str, _rel: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// A tool showed `text` to the agent as the content of `rel`, as it is on disk now. A workspace
    /// may keep it to recognise a later quote of that version; the default forgets it.
    fn note_seen(&self, _rel: &str, _text: &str) {}

    /// Where the latest sessions stopped: the files they changed and the work they left open. A
    /// workspace without a ledger has none.
    fn work_front(&self) -> Front {
        Front::default()
    }
}

/// Where the work of the latest sessions stopped, read from the ledger.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Front {
    /// The files the latest sessions changed, each once, the most recently changed first.
    pub moved: Vec<Moved>,
    /// The handover tasks still standing and the notes of the latest sessions, the most recent
    /// turn first.
    pub open: Vec<OpenItem>,
}

/// A file that a turn of the latest sessions changed, and who did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    /// The path below the root, `/`-separated.
    pub path: String,
    /// The last turn that changed it, as `SS.TT`.
    pub turn: String,
    /// How many turns that recorded something came after that turn, the active one included.
    pub age: usize,
    /// The ledger noticed the change on disk (`[External disk edit]`); the agent did not make it.
    pub outside: bool,
}

/// What an [`OpenItem`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Task,
    Note,
}

/// A handover task or a note of the latest sessions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenItem {
    /// `SS.TT.task.NN` or `SS.TT.note.NN`.
    pub id: String,
    pub kind: ItemKind,
    pub text: String,
}

/// The root of the registered project, if there is one.
pub fn root_of(workspace: Option<&dyn Workspace>) -> Option<&Path> {
    workspace.map(|workspace| workspace.root())
}
