//! The mention graph of a survey: which definition mentions which, and how much the mention says.
//!
//! A definition `D` mentions the names in its signature and body (`FileFacts` keeps them as
//! [`Use`](crate::engine::dependencies::Use)s). The name is matched to the definitions called that
//! in the survey, with no resolution of types beyond three rules read from the code:
//!
//! - A call that says where it goes keeps only the definitions it can reach (`Resolver::reach`):
//!   `self.f()` and `Self::f()` reach the methods of the enclosing type, `x.f()` on a receiver of
//!   known type and `Type::f()` those of that type (its own and the default methods of the traits
//!   it implements), a call on a trait object those of the trait and its implementations, and
//!   `module::f()` or an import of `f` from `module` the definitions of a file or inline module of
//!   that name. A function that calls the name in several ways (`NoteId::parse(s)`,
//!   `TaskId::parse(s)`) reaches what each way reaches. A call narrowed this way is not shadowed by
//!   a definition of the same name in its own file: a wrapper `parse` that delegates to
//!   `other::parse` mentions that one.
//! - A call that is proved to leave the project mentions nothing: a path through the standard
//!   library or a dependency, imported or written without a `use` (`std::fs::read`,
//!   `serde_json::from_str`), a name that an import binds to one, a type or trait that the project
//!   does not declare (`Vec::new()`), or a type that has no such method in the project
//!   (`self.clone()`, derived). Where types inherit methods (`Calls::types_inherit_methods`, which
//!   each language answers) a type of the project is the exception: the method may come from a base
//!   class that the survey does not link to it, so finding none leaves the candidates open. It does
//!   not fall back to a same-named definition of the file or
//!   the project. A call that only fails to say (a receiver of unknown type, a module or an
//!   import that none of the candidates is in) leaves the candidates open for the rules below.
//! - A name that the file defines itself means those definitions: a file that has its own `parse`
//!   does not mention anyone else's.
//! - Otherwise a name that an import of the file binds to the standard library or a dependency
//!   (the `outside` test of [`Graph::build`]) mentions nothing of the project, and a name that the
//!   survey defines six or more times (`new`, `fmt`) says too little about where it goes: both are
//!   dropped. The count is taken after narrowing, so `new` called as `Store::new()` is not lost
//!   among the other `new`s of the project. The limit is derived from the data, there is no list
//!   of names.
//!
//! - A mention never crosses projects. The survey of a folder that holds several projects (each
//!   with its own manifest) matches names inside each project alone: a name that another project
//!   writes the same (`tempfile::TempDir` next to a `TempDir` of a test kit) is a name in common,
//!   not a use. A project that depends on a sibling project shows no edge to it.
//!
//! The weight of a mention is the rarity of the name: `1 / candidates`, so a name defined once is a
//! full edge and a name defined twice gives each definition half. A mention made inside test code
//! counts a quarter. A name counts once per definition, however many ways it is used there (as a
//! type and as a value). Weights of several names between the same two definitions add up.
//!
//! Every definition except an `impl` block and the kinds listed on request can be a target.
//! Whether it is `exported` is not asked: a crate's own spine is `pub(crate)`, which
//! [`Declarations::exported`](crate::language::Declarations::exported) deliberately leaves out.
//! `exported` decides only which definitions are doors.
//!
//! `Ca` is the weight that arrives at a definition (dependents), `Ce` the weight that leaves it
//! (dependencies), `I = Ce / (Ca + Ce)`. A file's load counts the mentions that cross files only.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use super::facts::{Def, FileFacts};
use crate::engine::index::{in_module_at, names_a_type};
use crate::engine::references::Role;
use crate::engine::scope::bare_name;
use crate::language::{CallKind, CallSite, Import, registry};

/// A name defined this many times in the survey tells nothing about where a mention goes.
const COMMON: usize = 6;
/// How much a mention made inside test code counts.
const TEST_SHARE: f64 = 0.25;
/// How much a function counts when a name is written as a plain value, not called: a variable
/// called `rows` is hardly a mention of a method `rows`.
const VALUE_OF_FUNCTION: f64 = 0.25;
/// A definition is a base when it is at most this unstable.
const BASE_LIMIT: f64 = 0.35;
/// A definition is a hub when it is at least this unstable.
const HUB_LIMIT: f64 = 0.65;

/// A definition of the survey: the index of its file and its index in [`FileFacts::defs`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefId {
    pub file: usize,
    pub def: usize,
}

/// One mention: the other end and the weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    /// The node at the other end.
    pub to: usize,
    pub weight: f64,
}

/// What arrives at a definition or file and what leaves it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Load {
    /// Weighted dependents.
    pub ca: f64,
    /// Weighted dependencies.
    pub ce: f64,
}

impl Load {
    /// `Ce / (Ca + Ce)`; `None` for a definition that nothing mentions and that mentions nothing.
    pub fn instability(self) -> Option<f64> {
        let total = self.ca + self.ce;
        (total > 0.0).then(|| self.ce / total)
    }
}

/// What a definition or file is for, read from its load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    /// Many depend on it and it depends on little: read it first.
    Base,
    /// It depends on much and has dependents: read it to see the flow.
    Hub,
    /// It can be named from outside, nothing in the survey mentions it, and it mentions something:
    /// execution starts here.
    Door,
    /// Everything else.
    Detail,
}

impl Tag {
    /// The word shown in a listing; empty for a detail.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Hub => "hub",
            Self::Door => "door",
            Self::Detail => "",
        }
    }
}

/// The mentions between the definitions of the files of a survey. A node is a definition, numbered
/// file by file in the order of [`FileFacts::defs`].
pub struct Graph {
    /// The first node of each file, and the number of nodes at the end.
    first: Vec<usize>,
    ids: Vec<DefId>,
    out: Vec<Vec<Edge>>,
    inn: Vec<Vec<Edge>>,
    loads: Vec<Load>,
    file_loads: Vec<Load>,
    /// Per node: it can be named from outside its file and is a definition that other code names.
    exported: Vec<bool>,
    file_exported: Vec<bool>,
    /// The distinct names behind the mentions from each file to each other file.
    file_names: BTreeMap<(usize, usize), BTreeSet<String>>,
}

impl Graph {
    /// The graph of `files`, as `Survey::files` lists them. `projects` holds the project number of
    /// each file; a mention goes to definitions of the same project only. `outside(file, name)`
    /// tells whether an import of file number `file` binds `name` to the standard library or a
    /// dependency. `written(file, root)` tells whether the first name of a path written in file
    /// number `file` (`std` in `std::fs::read`) is the standard library or a dependency, whether an
    /// import brings it in or the language lets the path stand alone.
    pub fn build(
        files: &[(String, Arc<FileFacts>)],
        projects: &[usize],
        outside: &dyn Fn(usize, &str) -> bool,
        written: &dyn Fn(usize, &str) -> bool,
    ) -> Self {
        debug_assert_eq!(projects.len(), files.len());
        let mut first = Vec::with_capacity(files.len() + 1);
        let mut ids = Vec::new();
        for (file, (_, facts)) in files.iter().enumerate() {
            first.push(ids.len());
            ids.extend((0..facts.defs.len()).map(|def| DefId { file, def }));
        }
        first.push(ids.len());
        let def_of = |id: DefId| &files[id.file].1.defs[id.def];

        let mut named: HashMap<(usize, &str), Vec<usize>> = HashMap::new();
        let mut exported = vec![false; ids.len()];
        let mut file_exported = vec![false; files.len()];
        for (node, id) in ids.iter().enumerate() {
            let def = def_of(*id);
            if !is_named(def) {
                continue;
            }
            named
                .entry((projects[id.file], bare_name(&def.symbol.name)))
                .or_default()
                .push(node);
            exported[node] = def.exported;
            file_exported[id.file] |= def.exported;
        }

        // The traits each type implements, by project: what lets `Type::f()` and `self.f()` reach a
        // default method that the trait defines and the type does not.
        let mut implemented: HashSet<(usize, &str, &str)> = HashSet::new();
        for id in &ids {
            let def = def_of(*id);
            let project = projects[id.file];
            if let Some(tr) = &def.symbol.trait_name {
                implemented.insert((project, bare_name(&def.symbol.name), bare_name(tr)));
            }
            for frame in def.symbol.scope.frames() {
                if let Some(tr) = &frame.trait_name {
                    implemented.insert((project, frame.bare_name(), bare_name(tr)));
                }
            }
        }
        let resolver = Resolver {
            files,
            ids: &ids,
            projects,
            named: &named,
            implemented: &implemented,
            outside,
            written,
        };

        let mut weights: BTreeMap<(usize, usize), f64> = BTreeMap::new();
        let mut file_names: BTreeMap<(usize, usize), BTreeSet<String>> = BTreeMap::new();
        for (node, id) in ids.iter().enumerate() {
            let def = def_of(*id);
            if def.uses.is_empty() {
                continue;
            }
            let share = if def.symbol.test || def.symbol.scope.in_test() {
                TEST_SHARE
            } else {
                1.0
            };
            let mut mentions: BTreeMap<&str, Mention<'_>> = BTreeMap::new();
            for used in &def.uses {
                let mention = mentions.entry(bare_name(&used.name)).or_default();
                mention.roles.push(used.role);
                mention.first = mention.first.or(used.site.as_ref());
            }
            // A function can call one name in several ways (`NoteId::parse(s)`, `TaskId::parse(s)`),
            // and `Use::site` keeps only the first: the distinct calls of the body hold them all.
            for call in &def.calls {
                if let Some(mention) = mentions.get_mut(bare_name(&call.site.name)) {
                    mention.sites.push(&call.site);
                }
            }
            for (name, mut mention) in mentions {
                let Some(defined) = named.get(&(projects[id.file], name)) else {
                    continue;
                };
                if mention.sites.is_empty() {
                    mention.sites.extend(mention.first);
                }
                let candidates: Vec<usize> = match resolver.reach(defined, &mention, *id) {
                    Reach::Only(found) if found.is_empty() || found.len() >= COMMON => continue,
                    Reach::Only(found) => found,
                    Reach::Open => {
                        let local: Vec<usize> = defined
                            .iter()
                            .copied()
                            .filter(|&target| ids[target].file == id.file)
                            .collect();
                        if !local.is_empty() {
                            local
                        } else if defined.len() >= COMMON || outside(id.file, name) {
                            continue;
                        } else {
                            defined.clone()
                        }
                    }
                };
                // The way the name is written must suit the definition: a type is named as a type.
                let targets: Vec<(usize, f64)> = candidates
                    .iter()
                    .map(|&target| {
                        let fit = mention
                            .roles
                            .iter()
                            .map(|&role| fit(role, def_of(ids[target])))
                            .fold(0.0, f64::max);
                        (target, fit)
                    })
                    .filter(|&(_, fit)| fit > 0.0)
                    .collect();
                for &(target, fit) in &targets {
                    if target != node && !encloses(files, ids[target], *id) {
                        *weights.entry((node, target)).or_default() +=
                            share * fit / targets.len() as f64;
                        if ids[target].file != id.file {
                            file_names
                                .entry((id.file, ids[target].file))
                                .or_default()
                                .insert(name.to_string());
                        }
                    }
                }
            }
        }

        let mut out = vec![Vec::new(); ids.len()];
        let mut inn = vec![Vec::new(); ids.len()];
        let mut loads = vec![Load::default(); ids.len()];
        let mut file_loads = vec![Load::default(); files.len()];
        for (&(source, target), &weight) in &weights {
            out[source].push(Edge { to: target, weight });
            inn[target].push(Edge { to: source, weight });
            loads[source].ce += weight;
            loads[target].ca += weight;
            let (from, to) = (ids[source].file, ids[target].file);
            if from != to {
                file_loads[from].ce += weight;
                file_loads[to].ca += weight;
            }
        }
        Self {
            first,
            ids,
            out,
            inn,
            loads,
            file_loads,
            exported,
            file_exported,
            file_names,
        }
    }

    /// How many files the graph was built from.
    pub fn files(&self) -> usize {
        self.file_loads.len()
    }

    /// How many definitions it holds.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The node of a definition.
    pub fn node(&self, id: DefId) -> usize {
        self.first[id.file] + id.def
    }

    /// The definition a node stands for.
    pub fn id(&self, node: usize) -> DefId {
        self.ids[node]
    }

    /// The nodes of the definitions of file number `file`.
    pub fn nodes_of(&self, file: usize) -> std::ops::Range<usize> {
        self.first[file]..self.first[file + 1]
    }

    /// What `node` mentions, by node number.
    pub fn uses(&self, node: usize) -> &[Edge] {
        &self.out[node]
    }

    /// What mentions `node`, by node number.
    pub fn used_by(&self, node: usize) -> &[Edge] {
        &self.inn[node]
    }

    pub fn load(&self, node: usize) -> Load {
        self.loads[node]
    }

    /// The load of a file, from the mentions that cross files.
    pub fn file_load(&self, file: usize) -> Load {
        self.file_loads[file]
    }

    /// The weight of the mentions from each file to each other file, as `(from, to)` pairs.
    pub fn file_edges(&self) -> BTreeMap<(usize, usize), f64> {
        let mut edges = BTreeMap::new();
        for (node, mentions) in self.out.iter().enumerate() {
            for edge in mentions {
                let (from, to) = (self.ids[node].file, self.ids[edge.to].file);
                if from != to {
                    *edges.entry((from, to)).or_default() += edge.weight;
                }
            }
        }
        edges
    }

    /// The distinct names behind the mentions from each file to each other file, as
    /// [`file_edges`](Self::file_edges) keys them: one name is more often a collision than a
    /// dependency.
    pub fn file_names(&self) -> &BTreeMap<(usize, usize), BTreeSet<String>> {
        &self.file_names
    }

    /// The tag of every definition, by node. The quartiles are those of the loads in the graph.
    pub fn def_tags(&self) -> Vec<Tag> {
        tags(&self.loads, &self.exported)
    }

    /// The tag of every file. A file is a door when it holds an exported definition and nothing
    /// in another file mentions it; a manifest entrance makes it one too, which the caller adds.
    pub fn file_tags(&self) -> Vec<Tag> {
        tags(&self.file_loads, &self.file_exported)
    }
}

/// Whether other code can mention the definition by name: not an `impl` block, which defines no
/// name, and not a kind listed only on request (a field, a variant).
pub fn is_named(def: &Def) -> bool {
    let kind = def.symbol.kind;
    !kind.on_request && !(kind.owner && !kind.declares_type)
}

/// Whether the definition is test code: marked as a test, or written inside one.
pub fn in_test(def: &Def) -> bool {
    def.symbol.test || def.symbol.scope.in_test()
}

/// How well the definition `def` suits a name written the way `role` says: a type is named as a
/// type, and a function named as a plain value is rarely the function (see [`VALUE_OF_FUNCTION`]).
/// `0.0` means it is not what the name means.
fn fit(role: Role, def: &Def) -> f64 {
    let kind = def.symbol.kind;
    match role {
        Role::Type if !kind.declares_type => 0.0,
        Role::Use if kind.function => VALUE_OF_FUNCTION,
        _ => 1.0,
    }
}

/// Whether the definition `outer` is written around the definition `inner`.
fn encloses(files: &[(String, Arc<FileFacts>)], outer: DefId, inner: DefId) -> bool {
    if outer.file != inner.file {
        return false;
    }
    let defs = &files[inner.file].1.defs;
    let mut at = defs[inner.def].parent;
    while let Some(parent) = at {
        if parent == outer.def {
            return true;
        }
        at = defs[parent].parent;
    }
    false
}

/// How one definition writes one name: the roles it has there and the ways it calls it.
#[derive(Default)]
struct Mention<'a> {
    roles: Vec<Role>,
    /// Every distinct call of the name in the body.
    sites: Vec<&'a CallSite>,
    /// The first call, as `Use::site` keeps it: all that is known of a name used outside a function.
    first: Option<&'a CallSite>,
}

/// Where a name written in one way can go among the definitions that have it.
enum Reach {
    /// The way it is written does not tell: a bare name, a call on a value of unknown type, a
    /// module or an import that none of the candidates is in. Every candidate stays open.
    Open,
    /// Only these definitions, none perhaps: the call goes outside the project, or to a type that
    /// has no such method in it.
    Only(Vec<usize>),
}

impl Reach {
    /// `Open` for a set that holds nothing: what the call names, a module for one, may be one that
    /// re-exports what it does not define.
    fn or_open(self) -> Self {
        match self {
            Self::Only(found) if found.is_empty() => Self::Open,
            other => other,
        }
    }
}

/// What narrowing the candidates of a name needs to know about the survey.
struct Resolver<'a> {
    files: &'a [(String, Arc<FileFacts>)],
    ids: &'a [DefId],
    projects: &'a [usize],
    /// The named definitions by project and bare name, as nodes.
    named: &'a HashMap<(usize, &'a str), Vec<usize>>,
    /// `(project, type, trait)` for every `impl Trait for Type`.
    implemented: &'a HashSet<(usize, &'a str, &'a str)>,
    outside: &'a dyn Fn(usize, &str) -> bool,
    written: &'a dyn Fn(usize, &str) -> bool,
}

/// The definitions among `defined` that one call can reach; `None` when it says nothing.
impl Resolver<'_> {
    /// What the way the name of `mention` is called by the definition `caller` says about the
    /// candidates `defined`. When the function calls the name in several ways, it reaches what each
    /// way reaches; one way that says nothing leaves every candidate open. A name that is also
    /// written as a type or as a plain value is not narrowed: the calls tell only about the calls.
    fn reach(&self, defined: &[usize], mention: &Mention<'_>, caller: DefId) -> Reach {
        if mention.sites.is_empty() || mention.roles.iter().any(|&role| role != Role::Call) {
            return Reach::Open;
        }
        let mut found: BTreeSet<usize> = BTreeSet::new();
        for site in &mention.sites {
            match self.reach_by(defined, site, caller) {
                Reach::Open => return Reach::Open,
                Reach::Only(targets) => found.extend(targets),
            }
        }
        Reach::Only(found.into_iter().collect())
    }

    /// What one call says about the candidates `defined`.
    fn reach_by(&self, defined: &[usize], site: &CallSite, caller: DefId) -> Reach {
        let (label, facts) = &self.files[caller.file];
        let project = self.projects[caller.file];
        let scope_of = |target: usize| {
            let id = self.ids[target];
            &self.files[id.file].1.defs[id.def].symbol.scope
        };
        let pick = |keep: &dyn Fn(usize) -> bool| {
            Reach::Only(
                defined
                    .iter()
                    .copied()
                    .filter(|&target| keep(target))
                    .collect(),
            )
        };
        // The methods of the type `ty`: its own, and the default methods of the traits it
        // implements. Where types inherit methods, one that the project declares may get the
        // method from a base class that the survey does not link to it, so finding none proves
        // nothing; a type that the project does not declare belongs to the standard library or a
        // dependency, and in a language without inheritance (Rust) finding none proves the call
        // leaves the project.
        let inherits =
            registry::for_path(label).is_none_or(|syntax| syntax.types_inherit_methods());
        let methods_of = |ty: &str| {
            let found = pick(&|target| {
                scope_of(target).owner().is_some_and(|owner| {
                    owner.bare_name() == ty
                        || (owner.kind.interface
                            && self.implemented.contains(&(project, ty, owner.bare_name())))
                })
            });
            match found {
                Reach::Only(targets)
                    if targets.is_empty()
                        && inherits
                        && self.named.contains_key(&(project, ty)) =>
                {
                    Reach::Open
                }
                other => other,
            }
        };
        let in_module = |module: &str| {
            pick(&|target| {
                in_module_at(
                    &self.files[self.ids[target].file].0,
                    scope_of(target),
                    module,
                )
            })
            .or_open()
        };
        // `fs::read(..)` and `std::fs::read(..)` go to the standard library, `serde_json::from_str`
        // to a dependency, with or without a `use` that says so: not to a project module or
        // function that happens to have the same name.
        if matches!(site.kind, CallKind::Path(_))
            && site
                .root
                .as_deref()
                .is_some_and(|root| (self.written)(caller.file, root))
        {
            return Reach::Only(Vec::new());
        }
        match &site.kind {
            CallKind::Own => {
                let Some(own) = facts.defs[caller.def]
                    .symbol
                    .scope
                    .frames()
                    .iter()
                    .rev()
                    .find(|frame| frame.is_owner())
                else {
                    return Reach::Open;
                };
                if own.kind.interface {
                    // A default method of a trait calls the trait's methods and those of its
                    // implementations.
                    pick(&|target| {
                        scope_of(target)
                            .owner()
                            .is_some_and(|owner| owner.implements(own.bare_name()))
                    })
                } else {
                    methods_of(own.bare_name())
                }
            }
            CallKind::MethodOf(ty) => methods_of(bare_name(ty)),
            CallKind::Dyn(tr) => {
                let tr = bare_name(tr);
                // A trait that the project does not declare belongs to the standard library or a
                // dependency (`dyn Iterator`).
                if !self.named.contains_key(&(project, tr)) {
                    return Reach::Only(Vec::new());
                }
                pick(&|target| {
                    scope_of(target)
                        .owner()
                        .is_some_and(|owner| owner.implements(tr))
                })
                .or_open()
            }
            CallKind::Path(segment) if names_a_type(label, segment) => {
                methods_of(bare_name(segment))
            }
            CallKind::Path(module) => in_module(module),
            CallKind::Free | CallKind::Any => {
                if (self.outside)(caller.file, &site.name) {
                    return Reach::Only(Vec::new());
                }
                match imported_module(label, &facts.imports, &site.name) {
                    Some(module) => in_module(module),
                    None => Reach::Open,
                }
            }
            CallKind::Method => Reach::Open,
        }
    }
}

/// The module that an import of the file takes `name` from (`other` for `use crate::other::name`).
/// `crate`, `self` and `super` name a place in the module tree, not a module.
fn imported_module<'a>(label: &str, imports: &'a [Import], name: &str) -> Option<&'a str> {
    let syntax = registry::for_path(label);
    imports
        .iter()
        .filter(|import| {
            import.local != "*" && import.path.last().map(String::as_str) == Some(name)
        })
        .filter_map(|import| import.path.iter().rev().nth(1))
        .map(String::as_str)
        .find(|module| !syntax.is_some_and(|syntax| syntax.is_path_keyword(module)))
}

fn tags(loads: &[Load], exported: &[bool]) -> Vec<Tag> {
    let dependents = upper_quartile(loads.iter().map(|load| load.ca));
    let dependencies = upper_quartile(loads.iter().map(|load| load.ce));
    loads
        .iter()
        .zip(exported)
        .map(|(load, &exported)| {
            let unstable = load.instability();
            if load.ca >= dependents && unstable.is_some_and(|i| i <= BASE_LIMIT) {
                Tag::Base
            } else if load.ca > 0.0
                && load.ce >= dependencies
                && unstable.is_some_and(|i| i >= HUB_LIMIT)
            {
                Tag::Hub
            } else if exported && load.ca == 0.0 && load.ce > 0.0 {
                Tag::Door
            } else {
                Tag::Detail
            }
        })
        .collect()
}

/// The tags of `loads` read against their own quartiles. `exported[i]` says whether item `i` can
/// be a door.
pub fn tag_loads(loads: &[Load], exported: &[bool]) -> Vec<Tag> {
    tags(loads, exported)
}

/// The smallest of the largest quarter of the positive `values`; infinite when there are none, so
/// that nothing reaches it.
fn upper_quartile(values: impl Iterator<Item = f64>) -> f64 {
    let mut positive: Vec<f64> = values.filter(|value| *value > 0.0).collect();
    if positive.is_empty() {
        return f64::INFINITY;
    }
    positive.sort_by(f64::total_cmp);
    positive[(positive.len() * 3 / 4).min(positive.len() - 1)]
}
