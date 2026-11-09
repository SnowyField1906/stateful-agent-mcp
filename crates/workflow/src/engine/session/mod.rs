//! Session/turn data model and its Markdown persistence under `.AGENTS/`: `format` is the file
//! format, `serialize` and `parse` write and read it.

mod format;
pub mod model;
pub mod parse;
pub(crate) mod seen;
pub mod serialize;
pub mod store;
