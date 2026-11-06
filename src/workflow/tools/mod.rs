//! The `workflow_*` tools, one file per tool named after it (`new_turn.rs` is
//! `workflow_new_turn`). Each file holds the tool's `DESCRIPTION` (what the agent is told), its
//! parameters and its handler.
//!
//! Types that several tools share (file, note and task operations, `GetItemsParams`) live in
//! `engine/` and are re-exported here, so every tool input is imported from this module.

pub mod get_files_diff;
pub mod get_files_history;
pub mod get_notes;
pub mod get_tasks;
pub mod get_turns;
pub mod new_turn;
pub mod register;
pub mod search;
pub mod write_files;
pub mod write_notes;
pub mod write_tasks;

pub use crate::workflow::engine::items::{GetItemsParams, NoteWriteOp, TaskWriteOp};
pub use crate::workflow::engine::ops::{FileOp, PatchHunk};
pub use get_files_diff::GetFilesDiffParams;
pub use get_files_history::GetFilesHistoryParams;
pub use get_turns::GetTurnsParams;
pub use new_turn::NewTurnParams;
pub(crate) use new_turn::new_turn_with_paths;
pub use register::RegisterParams;
pub use search::SearchParams;
pub use write_files::WriteFilesParams;
pub(crate) use write_files::write_files_with_paths;
pub use write_notes::WriteNotesParams;
pub use write_tasks::WriteTasksParams;
