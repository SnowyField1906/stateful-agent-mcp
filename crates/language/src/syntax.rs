//! The whole contract of a language.

use super::{Calls, Declarations, Grammar, Imports, Language, Packages};

/// Everything the code intelligence tools ask of one language: the five concern traits, and which
/// entry of the language catalog the language is. The tools only use this trait, so supporting
/// another language needs no change to them.
pub trait Syntax: Grammar + Declarations + Calls + Imports + Packages + Sync {
    /// The language this syntax describes, as listed in [`LANGUAGES`](super::LANGUAGES).
    fn language(&self) -> &'static Language;
}
