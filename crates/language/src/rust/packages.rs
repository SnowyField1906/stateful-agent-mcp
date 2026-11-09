//! Rust packages: `Cargo.toml` marks a project, and its dependencies say where a crate lives.

use std::path::{Path, PathBuf};

use crate::{Declarations, Origin, Package, Packages, locate, tables};

use super::RustSyntax;
use super::manifest;

impl Packages for RustSyntax {
    fn manifests(&self) -> &'static [&'static str] {
        &["Cargo.toml"]
    }

    fn declares_workspace(&self, dir: &Path) -> bool {
        tables::read(&dir.join("Cargo.toml"))
            .is_some_and(|manifest| tables::section(&manifest, &["workspace"]).is_some())
    }

    fn implicit_packages(&self) -> bool {
        true
    }

    fn package_origin(&self, package: &Package<'_>) -> Origin {
        let root = package.root;
        match root {
            // The crates of the standard library and the primitive types: names the language reserves.
            "std" | "core" | "alloc" | "proc_macro" | "test" | "bool" | "char" | "str" | "i8"
            | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
            | "usize" | "f32" | "f64" => Origin::Std,
            "crate" | "self" | "super" | "Self" => Origin::Project,
            // The manifest is the list of dependencies.
            _ => match manifest::find(root, package.from, package.project) {
                Some(manifest::Declared::Path(dir)) => locate(&dir, package.project, root),
                Some(manifest::Declared::Package) => Origin::Dependency(root.to_string()),
                // Not declared: a type (decided by the caller) or a module of the project.
                None if self.is_type_name(root) => Origin::Unknown,
                None => Origin::Project,
            },
        }
    }

    fn entrypoints(&self, dir: &Path) -> Vec<PathBuf> {
        let mut found = manifest::entrypoints(dir);
        for member in manifest::members(dir) {
            found.extend(manifest::entrypoints(&member));
        }
        found.sort();
        found.dedup();
        found
    }

    fn member_links(&self, dir: &Path) -> Vec<(PathBuf, PathBuf)> {
        manifest::member_links(dir)
    }
}
