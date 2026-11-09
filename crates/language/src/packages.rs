//! Packages: which manifests mark a project, and where the code that a name refers to comes from.

use std::fmt;
use std::path::{Path, PathBuf};

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

impl Origin {
    /// Whether the code comes from the standard library or a dependency, not from the project.
    pub fn is_outside(&self) -> bool {
        matches!(self, Self::Std | Self::Dependency(_))
    }
}

/// The tag of a call in the full listing: `std`, `dep serde_json`, `project` or `unresolved`.
impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Std => f.write_str("std"),
            Self::Dependency(package) => write!(f, "dep {package}"),
            Self::Project => f.write_str("project"),
            Self::Unknown => f.write_str("unresolved"),
        }
    }
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

    /// The files where execution of the project in `dir` starts or that form its public face, as
    /// absolute paths of files that exist, sorted. Only manifests and a few well-known folders are
    /// read.
    fn entrypoints(&self, _dir: &Path) -> Vec<PathBuf> {
        Vec::new()
    }

    /// The dependencies between the member projects of the workspace in `dir`, as their manifests
    /// declare them: pairs of (folder of the member, folder of the member it depends on), absolute,
    /// only between members. Empty for a folder that is no workspace.
    fn member_links(&self, _dir: &Path) -> Vec<(PathBuf, PathBuf)> {
        Vec::new()
    }
}
