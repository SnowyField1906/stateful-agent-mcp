//! Workflow scope: sessions, turns, notes, handover tasks and recorded file changes.
//!
//! - `tools/`: one file per `workflow_*` tool, holding its `DESCRIPTION`, parameters and handler.
//! - `engine/`: the [`Project`] and what it is built from: the file-operation engine (`ops`), the
//!   session data model and its Markdown storage (`session`), selectors and item helpers, and the
//!   usage-guide texts seeded into `.AGENTS/`.

mod engine;
pub mod tools;

pub use engine::Project;
