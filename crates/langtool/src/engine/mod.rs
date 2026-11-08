//! The langtool engine: scanning the supported files below a path, and the Tree-sitter analysis
//! on top of it (symbols, references, call graph, folded views). It knows a language only through
//! [`crate::language::Syntax`]; the configured commands live in [`crate::toolchain`].

pub mod aliases;
pub mod callgraph;
pub mod calltree;
pub mod definitions;
pub mod dependencies;
pub mod fold;
pub mod imports;
pub mod incoming;
pub mod index;
pub mod name_match;
pub mod origins;
pub mod outgoing;
pub mod outside;
pub mod reader;
pub mod references;
pub mod scan;
pub mod scope;
pub mod selector;
pub mod symbols;
