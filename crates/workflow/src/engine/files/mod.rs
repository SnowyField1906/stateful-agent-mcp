//! The batch core of `workflow_write_files`: applies file operations atomically and records the
//! turn's net changes.
//!
//! A batch is applied to an in-memory `Overlay`; the disk is touched only when every operation
//! succeeded. For the active turn the project keeps `active_baseline` (the content of each
//! touched file before the turn first touched it) and `active_moves` (`to_path -> from_path`).
//! After every batch the turn's `updates` are recomputed as the net difference between that
//! baseline and the files on disk, `.AGENTS/.deleted/` is synced with any net-deleted files, and
//! `.AGENTS/.known/` takes the content of every touched file (see `external`).
//!
//! - `batch`: running a batch on the overlay, then committing it and recording its net result in
//!   the active turn.
//! - `miss`: where a failed `Patch` was aimed, for the hint after a repeated miss.
//! - `normalize`: validating and spelling the paths of each operation.
//! - `op`: applying one operation to the overlay.
//! - `recover`: `Recover`, which restores files from past turns.

mod batch;
mod miss;
mod normalize;
mod op;
mod recover;

pub(crate) use batch::Batch;
pub use miss::PatchMiss;
