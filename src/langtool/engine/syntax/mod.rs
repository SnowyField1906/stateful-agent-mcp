//! Per-language syntax: [`Syntax`] describes a grammar, one module per language implements it,
//! and [`registry`] maps a [`crate::toolchain::Language`] to its syntax.

pub mod python;
pub mod registry;
pub mod rust;
mod scope;
mod spec;

pub use scope::{Frame, Lineage, is_test_scope};
pub use spec::{
    Alias, CallKind, CallSite, Definition, SymbolSelector, Syntax, header, line_span, node_text,
    parse, unit_span,
};
