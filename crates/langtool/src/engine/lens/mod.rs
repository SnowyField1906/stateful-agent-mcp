//! The lens: what the orientation tools know about the code of a scan, read once per file.
//!
//! - `facts`: everything one file holds, from a single parse: definitions with the names they
//!   use and the calls they make, the identifiers, the imports and the file's documentation.
//! - `store`: the facts of files kept between calls, keyed by what the disk says about the file,
//!   and a survey of the files below a scan target.
//! - `graph`: the weighted mentions between the definitions of a survey, their loads (`Ca`, `Ce`)
//!   and the roles read from them (base, hub, door).
//!
//! Only the [`Syntax`](crate::language::Syntax) trait is used, so no language is named here.

pub mod facts;
pub mod graph;
pub mod store;
