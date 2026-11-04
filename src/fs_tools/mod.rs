//! Read-only filesystem tools: listing, finding, reading, searching and inspecting files.
//!
//! They respect `.gitignore` (unless `show_gitignored` is set), never show `.git` or `.AGENTS`, stay inside
//! the project root unless a path is given as `path_absolute` (which reads straight from disk
//! and needs no registered project), and return compact plain text.

mod calls;
mod find;
mod fmt;
mod grep;
mod info;
mod ls;
mod read;
mod references;
mod resolve;
mod scan;
mod symbols;
mod walk;

pub use calls::code_calls;
pub use find::find;
pub use grep::grep;
pub use info::file_info;
pub use ls::ls;
pub use read::read_files;
pub use references::code_references;
pub use symbols::symbols;
