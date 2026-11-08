//! Python: Tree-sitter support, one file per concern of the contract.

mod calls;
mod declarations;
mod grammar;
mod imports;
mod packages;
mod stdlib;

use crate::language::{Language, PYTHON, Syntax};

pub struct PythonSyntax;

impl Syntax for PythonSyntax {
    fn language(&self) -> &'static Language {
        &PYTHON
    }
}
