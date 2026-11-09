//! The lens: what the orientation tools know about the code of a scan, read once per file.
//!
//! - `facts`: everything one file holds, from a single parse: definitions with the names they
//!   use and the calls they make, the identifiers, the imports and the file's documentation.
//! - `store`: the facts of files kept between calls, keyed by what the disk says about the file,
//!   and a survey of the files below a scan target.
//! - `graph`: the weighted mentions between the definitions of a survey, their loads (`Ca`, `Ce`)
//!   and the roles read from them (base, hub, door).
//! - `codebase`: a survey with its graph and the tag of every definition and file.
//! - `layers`: the units of a codebase and the line of what rests on what.
//! - `concept`: the definitions that the words of a query point at.
//! - `slice`: the definitions around some seeds, and the shortest way in from the outside.
//! - `words`: the subwords of names and the few that tell a file from the others.
//! - `zoom`: the one word that says how many rows an overview spends.
//!
//! Only the [`Syntax`](crate::language::Syntax) trait is used, so no language is named here.

pub mod codebase;
pub mod concept;
pub mod facts;
pub mod graph;
pub mod layers;
pub mod slice;
pub mod store;
pub mod words;
pub mod zoom;
