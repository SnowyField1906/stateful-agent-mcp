//! Rust: Tree-sitter support, one file per concern of the contract.

mod bindings;
mod bounds;
mod calls;
mod declarations;
mod grammar;
mod imports;
mod manifest;
mod nodes;
mod packages;
mod receiver;
mod types;

use crate::{Language, RUST, Syntax};

pub struct RustSyntax;

impl Syntax for RustSyntax {
    fn language(&self) -> &'static Language {
        &RUST
    }
}
