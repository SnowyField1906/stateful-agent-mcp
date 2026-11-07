//! The workflow engine: the [`Project`] and the logic and types the tools build on. It never
//! imports `tools/`.
//!
//! - `ops`: file operations (`FileOp`), the atomic overlay, patching, diffing, net changes.
//! - `session`: turns, notes and tasks as data, and their Markdown storage under `.AGENTS/`.
//! - `project`: the loaded [`Project`]; `selector` and `items`: selectors, and what notes, tasks
//!   and turn metadata share; `instructions`: the usage-guide texts.
//! - `external`, `files`, `notes`, `tasks`, `turns`: what the mutating tools do to a `Project`
//!   (record edits made outside the ledger, apply a file batch, note ops, task ops; open, store
//!   or abort a turn). A tool file adds only its params,
//!   its JSON output and a thin handler, so tool files never call each other.
//!
//! Dependency order, lowest first: `ops`, `session`, `project`, then `external`, `files`, `notes`,
//! `tasks`, `turns`.

pub mod external;
pub mod files;
pub mod instructions;
pub mod items;
pub mod notes;
pub mod ops;
pub mod project;
pub mod selector;
pub mod session;
pub mod tasks;
pub mod turns;

pub use project::Project;
