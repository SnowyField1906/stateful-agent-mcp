//! Language support: the languages the server knows, and what a language with code intelligence
//! provides to the tools.
//!
//! The tools in `langtool` and `workflow` never name a language; all they know about one comes
//! through [`Syntax`], which they find with [`registry`]. `Syntax` is five small traits, one per
//! concern, so a language is written as one file per concern and each trait lists exactly what that
//! concern needs:
//!
//! | Trait | Concern | Methods without a default |
//! |---|---|---|
//! | [`Grammar`] | parsing the source and showing it folded | `grammar`, `block_kinds`, `elision`, `identifier`, `text_kinds` |
//! | [`Declarations`] | what the source defines | `kinds`, `definition` |
//! | [`Calls`] | what a call calls and how it was written | `call_name` |
//! | [`Imports`] | what an import binds, and the module tree | none |
//! | [`Packages`] | where a package comes from | none |
//!
//! The data a trait produces is declared next to it ([`Definition`], [`CallSite`], [`Import`],
//! [`Origin`]). A default method describes a language without that feature, so a language only
//! writes what it has.
//!
//! Layout: a trait and its implementations share a file name, so `calls.rs` here is the trait and
//! `rust/calls.rs` is Rust's `impl`. [`catalog`] lists the languages by name and file extension,
//! [`registry`] the ones that have a `Syntax`; `tree`, `project` and `tables` are helpers that work
//! for any language.
//!
//! Adding a language:
//!
//! 1. Declare it in [`catalog`]: its name and file extensions, and add it to `LANGUAGES`.
//! 2. Add `<name>/mod.rs` with the unit struct and `impl Syntax` (which only says which catalog
//!    entry it is), and beside it the files `grammar.rs`, `declarations.rs`, `calls.rs`,
//!    `imports.rs` and `packages.rs`, each the `impl` of the trait of the same name in this folder.
//!    A helper belongs to the concern that uses it (`rust/receiver.rs` for calls), or to `nodes.rs`
//!    when several concerns use it.
//! 3. Declare `mod <name>;` here and add the struct to `SYNTAXES` in [`registry`].

pub mod catalog;
pub mod registry;

mod calls;
mod declarations;
mod grammar;
mod imports;
mod packages;
mod project;
mod python;
mod rust;
mod syntax;
mod tables;
mod tree;

pub use calls::{CallKind, CallSite, Calls};
pub use catalog::{LANGUAGES, Language, PYTHON, RUST, language_for_path};
pub use declarations::{Declarations, Definition, Kind, is_capitalized, is_conventional_test_path};
pub use grammar::{Grammar, Ident};
pub use imports::{Alias, Import, Imports};
pub use packages::{Origin, Package, Packages};
pub use project::{locate, project_root};
pub use syntax::Syntax;
pub use tree::{header, line_span, node_text, parse, unit_span};
