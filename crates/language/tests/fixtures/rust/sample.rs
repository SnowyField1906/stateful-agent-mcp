//! A sample of Rust for the contract tests in `tests/`.

use std::collections::*;
use std::fmt::Result as FmtResult;
use std::{fs, io};

/// A store of values.
pub struct Store {
    values: Vec<String>,
}

pub trait Putter {
    fn put(&self);
}

impl Store {
    /// Make an empty store.
    pub fn new() -> Self {
        Self { values: Vec::new() }
    }

    pub fn put(&self) {
        self.log();
        helper();
        Store::new();
        let other = Store::new();
        other.put();
    }

    fn log(&self) {}
}

fn helper() {
    // Read a file.
    let _ = fs::read("path");
}
