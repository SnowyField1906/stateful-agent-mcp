//! Imports: the names an import binds, and the module tree those names live in.

use tree_sitter::Node;

/// An import that gives a name another name in the importing file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    /// The name used in the file.
    pub alias: String,
    /// The last segment of what was imported: `b` for `use a::b as c`.
    pub original: String,
}

/// A name that an import statement binds in its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    /// The name used in the file; `*` for a wildcard import.
    pub local: String,
    /// The imported path, outermost segment first: `std`, `fs` for `use std::fs`, and `numpy` for
    /// `import numpy as np`. For a wildcard, the path of the module whose names it brings in.
    pub path: Vec<String>,
    /// A relative import (`from . import x`).
    pub relative: bool,
}

/// The imports of a language. A language without imports implements none of it.
pub trait Imports {
    /// Node kinds of import statements.
    fn import_kinds(&self) -> &'static [&'static str] {
        &[]
    }

    /// The names that the import statement `node` binds, one [`Import`] each. Only `node` itself
    /// is looked at: the caller visits every node of the tree.
    fn imports(&self, _node: Node<'_>, _source: &str) -> Vec<Import> {
        Vec::new()
    }

    /// The alias that `node` declares, if it is an aliased import. Imports without `as` are not
    /// aliases.
    fn alias(&self, _node: Node<'_>, _source: &str) -> Option<Alias> {
        None
    }

    /// Whether `segment` of an import path names a place in the module tree instead of a module:
    /// `crate`, `self` and `super`.
    fn is_path_keyword(&self, _segment: &str) -> bool {
        false
    }

    /// File stems that stand for the folder they are in: `mod` for `a/mod.rs`, which is the
    /// module `a`, and `__init__` for `a/__init__.py`.
    fn module_entry_stems(&self) -> &'static [&'static str] {
        &[]
    }
}
