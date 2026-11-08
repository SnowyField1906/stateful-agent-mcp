//! The registered project, as the tools below `workflow` see it.
//!
//! The read tools of `filesystem` and `langtool` need three things from the project: its root, the
//! last turn that changed a file, and a file as of a past turn. [`Workspace`] names them, so those
//! scopes depend on this trait and not on `workflow::Project`, which implements it.

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
}

/// The root of the registered project, if there is one.
pub fn root_of(workspace: Option<&dyn Workspace>) -> Option<&Path> {
    workspace.map(|workspace| workspace.root())
}
