//! Packages: which manifests mark a project, and where the code that a name refers to comes from.

use std::path::Path;

/// Where the code that a name refers to comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The project itself.
    Project,
    /// The language's standard library, prelude and builtins.
    Std,
    /// A package whose files are outside the project directory or ignored by `.gitignore`; the
    /// name is the package.
    Dependency(String),
    /// Not known.
    Unknown,
}

/// A package named in code, and where the code that names it lives.
pub struct Package<'a> {
    /// The first segment of an import path, or of a path written in code: `serde_json` for
    /// `serde_json::to_string(..)`.
    pub root: &'a str,
    /// Imported relatively (`from . import x`).
    pub relative: bool,
    /// The file that names it, absolute.
    pub from: &'a Path,
    /// The root of the project that file belongs to, absolute: the nearest folder with a manifest
    /// of this language (see [`Packages::manifests`]).
    pub project: &'a Path,
}

/// The packages of a language. A language without any implements none of it.
pub trait Packages {
    /// File names that make their folder the root of a project: `Cargo.toml`.
    fn manifests(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether the manifest in `dir` lists member projects, which then form one project with it
    /// (a Cargo or uv workspace).
    fn declares_workspace(&self, _dir: &Path) -> bool {
        false
    }

    /// Whether a path written in code can start with the name of a package that no import
    /// brought in (Rust's `serde_json::to_string(..)`). Python needs an `import` first.
    fn implicit_packages(&self) -> bool {
        false
    }

    /// Where `package` comes from. This is the only part of the contract that touches the disk,
    /// and only to find manifests and package folders; code of other files is never read.
    /// `Unknown` when the language cannot tell.
    fn package_origin(&self, _package: &Package<'_>) -> Origin {
        Origin::Unknown
    }
}
