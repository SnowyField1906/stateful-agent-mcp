//! Where a call goes when no indexed function answers to it.
//!
//! Such a call leaves the scanned code. What it leaves for is read from how its file got the
//! name, not from a list of well-known names:
//!
//! 1. A path or dotted chain (`serde_json::to_string`, `fs::read`, `np.array`) starts with a
//!    name that an import binds or, in a language that allows it, that names a package.
//! 2. An imported function or type (`use a::f`, `from a import f`).
//! 3. A name that no import brings in, in a file without a wildcard import, comes from the
//!    prelude or the builtins.
//!
//! The package at the head of the import is placed by the language
//! ([`Syntax::package_origin`]): the standard library, a declared dependency, or project code.
//! Files outside the project directory or ignored by `.gitignore` are not project code, so a
//! virtual environment, `node_modules` or the cargo registry need no list of names.
//!
//! The project is the nearest manifest root above the file ([`project_root`]), not the directory
//! that file names are relative to: a sibling project below the same directory is a dependency,
//! and an indexed definition in it stays a possible target of a call into it.
//!
//! A method called on a value of unknown type is not linked to a project method when the same
//! name is called elsewhere in the scan on a receiver known to come from the standard library or
//! a dependency (`x.push()` next to `names.push()` with `names: Vec<_>`).
//!
//! A type is the project's when a scanned file declares it or the project gives it methods of its
//! own (an inherent `impl`, a class, a trait). `impl From<A> for Vec<B>` declares nothing: it
//! leaves `Vec` where it was, so a `Vec` receiver stays the standard library's.

use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::imports::{bound, has_wildcard};
use super::index::{Entry, in_module};
use crate::language::{
    CallKind, CallSite, Import, Origin, Package, Syntax, project_root, registry,
};

/// What is known about where a call goes.
pub struct Verdict {
    pub origin: Origin,
    /// Decided by an import or a path written in the code, not by the absence of any other
    /// explanation.
    pub certain: bool,
}

fn certain(origin: Origin) -> Verdict {
    Verdict {
        origin,
        certain: true,
    }
}

fn guess(origin: Origin) -> Verdict {
    Verdict {
        origin,
        certain: false,
    }
}

fn known(origin: Origin) -> Option<Origin> {
    match origin {
        Origin::Unknown => None,
        other => Some(other),
    }
}

/// Places calls. `root` is the directory that file names are relative to and `index` the scanned
/// functions; the origin of each package of each file is looked up once.
pub struct Origins<'a> {
    root: &'a Path,
    index: &'a [Entry],
    /// Names of the types and traits that the scanned code declares or gives methods of its own.
    owners: HashSet<String>,
    /// `(type or trait, method)` pairs that the scanned code defines.
    defined: HashSet<(String, String)>,
    packages: RefCell<HashMap<(String, String, bool), Origin>>,
    /// The project root of each folder that holds a scanned file, per language: each language
    /// has its own manifests.
    projects: RefCell<HashMap<(PathBuf, &'static str), PathBuf>>,
    /// Names of methods called on a receiver known to come from outside the project.
    outside_methods: OnceCell<HashSet<String>>,
}

impl<'a> Origins<'a> {
    pub fn new(root: &'a Path, index: &'a [Entry]) -> Self {
        let mut owners = HashSet::new();
        let mut defined = HashSet::new();
        let mut listed = HashSet::new();
        for entry in index {
            // The functions of a file share one list of the types it declares.
            if listed.insert(Arc::as_ptr(&entry.func.types)) {
                owners.extend(entry.func.types.iter().cloned());
            }
            let Some(owner) = entry.func.scope.owner() else {
                continue;
            };
            // `impl Trait for Type` does not say where `Type` is declared.
            if owner.trait_name.is_none() {
                owners.insert(owner.bare_name().to_string());
            }
            defined.insert((owner.bare_name().to_string(), entry.func.name.clone()));
            if let Some(name) = &owner.trait_name {
                defined.insert((bare(name).to_string(), entry.func.name.clone()));
            }
        }
        Self {
            root,
            index,
            owners,
            defined,
            packages: RefCell::default(),
            projects: RefCell::default(),
            outside_methods: OnceCell::new(),
        }
    }

    /// Where `call`, made by `caller`, goes.
    pub fn verdict(&self, call: &CallSite, caller: &Entry) -> Verdict {
        self.place(call, &caller.file, &caller.func.imports)
    }

    /// Where `call`, made in `file` whose imports are `imports`, goes.
    pub fn place(&self, call: &CallSite, file: &str, imports: &[Import]) -> Verdict {
        let Some(syntax) = registry::for_path(file) else {
            return guess(Origin::Unknown);
        };
        if let Some(root) = call.root.as_deref()
            && call.kind != CallKind::Own
            && let Some(origin) = self.written(syntax, file, imports, root)
        {
            return certain(origin);
        }
        match &call.kind {
            CallKind::Free | CallKind::Any => {
                match self.imported(syntax, file, imports, &call.name) {
                    Some(origin) => certain(origin),
                    None if has_wildcard(imports) => guess(Origin::Unknown),
                    None if syntax.is_type_name(&call.name) => {
                        self.of_type(syntax, file, imports, &call.name)
                    }
                    None => guess(Origin::Std),
                }
            }
            CallKind::MethodOf(ty) | CallKind::Dyn(ty) => self.of_type(syntax, file, imports, ty),
            CallKind::Path(segment) if syntax.is_type_name(segment) => {
                self.of_type(syntax, file, imports, segment)
            }
            CallKind::Path(_) | CallKind::Own | CallKind::Method => guess(Origin::Unknown),
        }
    }

    /// The part of `defs`, the indexed definitions that `call` can reach by name, that it still
    /// reaches once its origin is known. An import or a written path that puts the callee in the
    /// standard library rules out every project function of that name. One that puts it in a
    /// dependency rules out the functions of the caller's own project, not those of another
    /// project, which is what a dependency on a sibling project reaches.
    pub fn narrow<'d>(
        &self,
        call: &CallSite,
        caller: &Entry,
        defs: Vec<&'d Entry>,
    ) -> Vec<&'d Entry> {
        if defs.is_empty() {
            return defs;
        }
        let verdict = self.verdict(call, caller);
        if !verdict.certain {
            return defs;
        }
        match verdict.origin {
            Origin::Std => Vec::new(),
            Origin::Dependency(_) => {
                let own = self.project(&caller.file);
                defs.into_iter()
                    .filter(|def| self.project(&def.file) != own)
                    .collect()
            }
            Origin::Project => in_imported_module(call, caller, defs),
            Origin::Unknown => defs,
        }
    }

    /// Whether `call` is `value.f()` on a value of unknown type while `f` is also called elsewhere
    /// in the scan on a receiver known to come from the standard library or a dependency. Such a
    /// call is more likely that one than a project method that happens to share the name.
    pub fn is_outside_method(&self, call: &CallSite) -> bool {
        call.kind == CallKind::Method && self.outside_methods().contains(&call.name)
    }

    /// Whether `call` is a method of a receiver typed as something the standard library or a
    /// dependency owns. The project can add methods to such a type through a trait, and those are
    /// found by name and type; any other method of that name is not one of the project's.
    pub fn is_outside_type(&self, call: &CallSite, caller: &Entry) -> bool {
        matches!(call.kind, CallKind::MethodOf(_) | CallKind::Dyn(_))
            && self.verdict(call, caller).origin.is_outside()
    }

    /// Where `name`, written in `file` whose imports are `imports`, comes from when an import binds
    /// it; `None` for a name that no import brings in or whose package is not known.
    pub fn name_origin(&self, file: &str, imports: &[Import], name: &str) -> Option<Origin> {
        let syntax = registry::for_path(file)?;
        self.imported(syntax, file, imports, name)
    }

    /// The names of the methods that some call in the scan makes on a typed receiver, or through a
    /// written package path, that the standard library or a dependency owns.
    fn outside_methods(&self) -> &HashSet<String> {
        self.outside_methods.get_or_init(|| {
            let mut names = HashSet::new();
            for entry in self.index {
                for call in &entry.func.calls {
                    let site = &call.site;
                    // A method that the project defines for that very type is the project's.
                    if is_method(&site.kind)
                        && !names.contains(&site.name)
                        && !self.defines_method(site)
                        && self.verdict(site, entry).origin.is_outside()
                    {
                        names.insert(site.name.clone());
                    }
                }
            }
            names
        })
    }

    /// Whether the project defines the method that `site` calls for the very type of its receiver.
    fn defines_method(&self, site: &CallSite) -> bool {
        matches!(
            &site.kind,
            CallKind::MethodOf(ty) | CallKind::Dyn(ty)
                if self.defined.contains(&(ty.clone(), site.name.clone()))
        )
    }

    /// A type or trait named in a receiver or a path: imported, defined in the scanned code, or
    /// else from the prelude or the builtins.
    fn of_type(&self, syntax: &dyn Syntax, file: &str, imports: &[Import], ty: &str) -> Verdict {
        let head = ty.split(['.', ':']).next().unwrap_or(ty);
        if let Some(origin) = self.imported(syntax, file, imports, head) {
            return certain(origin);
        }
        guess(if self.owners.contains(ty) {
            Origin::Project
        } else if has_wildcard(imports) {
            Origin::Unknown
        } else {
            Origin::Std
        })
    }

    /// The origin of `name` when an import binds it.
    fn imported(
        &self,
        syntax: &dyn Syntax,
        file: &str,
        imports: &[Import],
        name: &str,
    ) -> Option<Origin> {
        let import = bound(imports, name)?;
        let head = import.path.first()?;
        known(self.package(syntax, file, head, import.relative))
    }

    /// The origin of the first name of a written path: imported, or a package named directly in
    /// languages that allow it.
    fn written(
        &self,
        syntax: &dyn Syntax,
        file: &str,
        imports: &[Import],
        root: &str,
    ) -> Option<Origin> {
        self.imported(syntax, file, imports, root).or_else(|| {
            if syntax.implicit_packages() {
                known(self.package(syntax, file, root, false))
            } else {
                None
            }
        })
    }

    fn package(&self, syntax: &dyn Syntax, file: &str, name: &str, relative: bool) -> Origin {
        let key = (file.to_string(), name.to_string(), relative);
        if let Some(found) = self.packages.borrow().get(&key) {
            return found.clone();
        }
        let from = self.root.join(file);
        let project = self.project(file);
        let origin = syntax.package_origin(&Package {
            root: name,
            relative,
            from: &from,
            project: &project,
        });
        self.packages.borrow_mut().insert(key, origin.clone());
        origin
    }

    /// The root of the project that `file` belongs to: the workspace or nearest manifest of its
    /// language, else the root of the scan.
    pub fn project(&self, file: &str) -> PathBuf {
        let Some(syntax) = registry::for_path(file) else {
            return self.root.to_path_buf();
        };
        let path = self.root.join(file);
        let folder = path.parent().unwrap_or(self.root).to_path_buf();
        let key = (folder, syntax.language().name);
        if let Some(found) = self.projects.borrow().get(&key) {
            return found.clone();
        }
        let found = project_root(syntax, &path, self.root);
        self.projects.borrow_mut().insert(key, found.clone());
        found
    }
}

/// A function that an import takes from a module of the project (`use super::location::locate`)
/// is that module's, not a same-named function of another module. A module can re-export what it
/// does not define, so when none of `defs` is in the module they all stay.
fn in_imported_module<'d>(call: &CallSite, caller: &Entry, defs: Vec<&'d Entry>) -> Vec<&'d Entry> {
    if !matches!(call.kind, CallKind::Free | CallKind::Any) {
        return defs;
    }
    let syntax = registry::for_path(&caller.file);
    let module = caller
        .func
        .imports
        .iter()
        .filter(|import| import.local != "*" && import.path.last() == Some(&call.name))
        .filter_map(|import| import.path.iter().rev().nth(1))
        .find(|module| !syntax.is_some_and(|syntax| syntax.is_path_keyword(module)));
    let Some(module) = module else {
        return defs;
    };
    let inside: Vec<&Entry> = defs
        .iter()
        .copied()
        .filter(|def| in_module(def, module))
        .collect();
    if inside.is_empty() { defs } else { inside }
}

/// A type or trait name without its generic arguments: `From` for `From<Params>`.
fn bare(name: &str) -> &str {
    name.split('<').next().unwrap_or(name).trim()
}

/// Whether a call is made on a receiver, typed or not.
fn is_method(kind: &CallKind) -> bool {
    matches!(
        kind,
        CallKind::Method | CallKind::MethodOf(_) | CallKind::Dyn(_)
    )
}
