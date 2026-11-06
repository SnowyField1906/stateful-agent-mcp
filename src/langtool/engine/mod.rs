//! The langtool engine: scanning the supported files below a path, and the Tree-sitter analysis
//! on top of it (symbols, references, call graph, folded views). A language plugs in through
//! [`Syntax`] (see `syntax/`). The configured commands live in [`crate::toolchain`].

pub mod aliases;
pub mod callgraph;
pub mod fold;
pub mod name_match;
pub mod references;
pub mod scan;
pub mod symbols;
pub mod syntax;

pub use syntax::Syntax;
