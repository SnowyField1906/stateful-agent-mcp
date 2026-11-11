//! A survey with its mention graph and the roles read from it: what the overview tools share.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::facts::{Def, FileFacts};
use super::graph::{Graph, Tag, in_test, is_named};
use crate::engine::origins::Origins;
use crate::engine::scope::bare_name;

/// A definition is not what its file is about when it is this short or shorter, unless it is a type
/// with methods: an accessor (`len`, `get`, `first`), a type alias (`Result`).
const ACCESSOR_LINES: usize = 4;
/// A function of this many lines or more does real work, which makes it a headline.
const SUBSTANTIAL_LINES: usize = 8;
/// A file that test code mentions is support code of tests when other code accounts for at most
/// this share of the weight of the mentions (a starting value, to tune on this repo and `serena`).
const STRAY_SHARE: f64 = 0.1;
/// A function that this share of the files or more use is a verb of the whole codebase (`push`,
/// `resolve`), not a part of one feature (a starting value, to tune on this repo and `serena`).
const COMMON_SHARE: f64 = 0.1;
/// The most that one file adds to the mentions of another file in `test_support`. Without a cap the
/// weight grows with the number of calls, so a crate with many tests and a thin production caller
/// (the server) reads as support code of its own tests.
const FILE_MENTION_CAP: f64 = 1.0;

pub struct Codebase {
    /// Each file's shown path and facts.
    pub files: Vec<(String, Arc<FileFacts>)>,
    /// The project of every file ([`Origins::project`]), numbered in the order they first appear.
    pub projects: Vec<usize>,
    pub graph: Graph,
    /// The tag of every definition, by node.
    pub def_tags: Vec<Tag>,
    /// The tag of every file.
    pub file_tags: Vec<Tag>,
}

impl Codebase {
    /// The graph of `files`. A name that an import of its file binds to the standard library or a
    /// dependency, as `origins` places it, mentions nothing of the project.
    ///
    /// A file that only test code mentions outside itself is support code of tests (a `testkit`
    /// crate, a fixture builder): its definitions are marked as test code and the graph is built
    /// again, because mentions made in test code weigh less.
    ///
    /// Mentions stay inside a project ([`Origins::project`]), so a scan of several projects reads
    /// each of them as a scan of it alone does.
    pub fn new(mut files: Vec<(String, Arc<FileFacts>)>, origins: &Origins<'_>) -> Self {
        let mut numbers = HashMap::new();
        let projects: Vec<usize> = files
            .iter()
            .map(|(label, _)| {
                let next = numbers.len();
                *numbers.entry(origins.project(label)).or_insert(next)
            })
            .collect();
        let mut graph = build_graph(&files, &projects, origins);
        let support = test_support(&files, &graph);
        if support.iter().any(|&file| file) {
            for (file, (_, facts)) in files.iter_mut().enumerate() {
                if support[file] {
                    for def in &mut Arc::make_mut(facts).defs {
                        def.symbol.test = true;
                    }
                }
            }
            graph = build_graph(&files, &projects, origins);
        }
        let def_tags = graph.def_tags();
        let file_tags = graph.file_tags();
        Self {
            files,
            projects,
            graph,
            def_tags,
            file_tags,
        }
    }

    pub fn label(&self, file: usize) -> &str {
        &self.files[file].0
    }

    pub fn facts(&self, file: usize) -> &FileFacts {
        &self.files[file].1
    }

    pub fn def(&self, node: usize) -> &Def {
        let id = self.graph.id(node);
        &self.files[id.file].1.defs[id.def]
    }

    pub fn file_of(&self, node: usize) -> usize {
        self.graph.id(node).file
    }

    /// `file:first..last` of a definition, or `file:line` when it is on one line.
    pub fn locator(&self, node: usize) -> String {
        let def = self.def(node);
        span(
            self.label(self.file_of(node)),
            def.symbol.first,
            def.symbol.last,
        )
    }

    /// Whether the definition is one that other code names and that is not test code.
    pub fn listed(&self, node: usize) -> bool {
        let def = self.def(node);
        is_named(def) && !in_test(def)
    }

    /// Whether the definition is a short function (`len`, `get`, `first`): code that uses it says
    /// little about what that code is about, and the file that holds it is not about it either.
    pub fn is_accessor(&self, node: usize) -> bool {
        let symbol = &self.def(node).symbol;
        !symbol.kind.declares_type && symbol.last + 1 - symbol.first <= ACCESSOR_LINES
    }

    /// Whether a tenth of the files or more use the definition. The graph matches by name, so a
    /// function named `push` collects every `Vec::push` call, and no slice is about it.
    pub fn is_common(&self, node: usize) -> bool {
        let mut users: Vec<usize> = self
            .graph
            .used_by(node)
            .iter()
            .map(|edge| self.file_of(edge.to))
            .collect();
        users.sort_unstable();
        users.dedup();
        users.len() as f64 >= self.files.len() as f64 * COMMON_SHARE
    }

    /// Whether the file holds test code only: every definition that other code can name is test
    /// code (a test folder, or support code of tests). An `impl` block or a field names nothing,
    /// so it does not count. Such a file is no door and no start, whatever it exports.
    pub fn is_test_file(&self, file: usize) -> bool {
        let mut named = self
            .facts(file)
            .defs
            .iter()
            .filter(|def| is_named(def))
            .peekable();
        named.peek().is_some() && named.all(in_test)
    }

    /// What a file is about: its listed definitions, the most telling first. A type or a function of
    /// 8 lines or more comes before a short function, and an accessor of 4 lines or less is left
    /// out; the dependents of an owner's methods count for the owner, because code that uses a
    /// type mostly calls its methods. Within a rank: by dependents, then dependencies, then
    /// position. A file that has nothing but accessors keeps them all.
    pub fn headline(&self, file: usize) -> Vec<usize> {
        let facts = self.facts(file);
        let mut owned: HashMap<&str, f64> = HashMap::new();
        for node in self.graph.nodes_of(file) {
            let def = self.def(node);
            if in_test(def) {
                continue;
            }
            if let Some(parent) = def.parent {
                let owner = &facts.defs[parent].symbol;
                if owner.kind.owner {
                    *owned.entry(bare_name(&owner.name)).or_default() += self.graph.load(node).ca;
                }
            }
        }
        let lines = |node: usize| {
            let symbol = &self.def(node).symbol;
            symbol.last + 1 - symbol.first
        };
        let score = |node: usize| {
            let symbol = &self.def(node).symbol;
            let methods = if symbol.kind.declares_type {
                owned.get(bare_name(&symbol.name)).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            self.graph.load(node).ca + methods
        };
        let rank = |node: usize| {
            let symbol = &self.def(node).symbol;
            // A type of 4 lines or less with no methods (an alias such as `Result`) says nothing
            // about its file, whatever number of definitions name it.
            let real_type = symbol.kind.declares_type
                && (lines(node) > ACCESSOR_LINES || owned.contains_key(bare_name(&symbol.name)));
            if real_type || lines(node) >= SUBSTANTIAL_LINES {
                0
            } else if lines(node) > ACCESSOR_LINES {
                1
            } else {
                2
            }
        };
        let mut nodes: Vec<usize> = self
            .graph
            .nodes_of(file)
            .filter(|&node| self.listed(node))
            .collect();
        if nodes.iter().any(|&node| rank(node) < 2) {
            nodes.retain(|&node| rank(node) < 2);
        }
        nodes.sort_by(|&a, &b| {
            rank(a)
                .cmp(&rank(b))
                .then(score(b).total_cmp(&score(a)))
                .then(self.graph.load(b).ce.total_cmp(&self.graph.load(a).ce))
                .then(self.def(a).symbol.first.cmp(&self.def(b).symbol.first))
        });
        nodes
    }
}

fn build_graph(
    files: &[(String, Arc<FileFacts>)],
    projects: &[usize],
    origins: &Origins<'_>,
) -> Graph {
    Graph::build(
        files,
        projects,
        &|file, name| {
            let (label, facts) = &files[file];
            origins
                .name_origin(label, &facts.imports, name)
                .is_some_and(|origin| origin.is_outside())
        },
        &|file, root| {
            let (label, facts) = &files[file];
            origins
                .written_origin(label, &facts.imports, root)
                .is_some_and(|origin| origin.is_outside())
        },
    )
}

/// The files that are not test code but that test code mentions from other files, and code that is
/// not test code hardly does: at most [`STRAY_SHARE`] of the weight of the mentions, where one file
/// adds at most [`FILE_MENTION_CAP`]. A word that a
/// helper shares with a local variable (`at`) is such a stray mention. A file that nothing mentions
/// is not one (it may be a door), and a mention from support code counts as a mention from test
/// code, so a helper of a helper is found too. Mentions of methods are not counted (see
/// [`in_impl`]).
fn test_support(files: &[(String, Arc<FileFacts>)], graph: &Graph) -> Vec<bool> {
    let mut support = vec![false; files.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for file in 0..files.len() {
            if support[file] || files[file].1.defs.iter().all(in_test) {
                continue;
            }
            let mut sources: BTreeMap<(usize, bool), f64> = BTreeMap::new();
            for node in graph.nodes_of(file) {
                if in_impl(&files[file].1, graph.id(node).def) {
                    continue;
                }
                for edge in graph.used_by(node) {
                    let from = graph.id(edge.to);
                    if from.file == file {
                        continue;
                    }
                    let test = support[from.file] || in_test(&files[from.file].1.defs[from.def]);
                    *sources.entry((from.file, test)).or_default() += edge.weight;
                }
            }
            let (mut from_tests, mut from_code) = (0.0, 0.0);
            for (&(_, test), &weight) in &sources {
                let weight = weight.min(FILE_MENTION_CAP);
                if test {
                    from_tests += weight;
                } else {
                    from_code += weight;
                }
            }
            if from_tests > 0.0 && from_code <= STRAY_SHARE * (from_tests + from_code) {
                support[file] = true;
                changed = true;
            }
        }
    }
    support
}

/// Whether the definition is a method, written in an `impl` block. Code calls it by a name that
/// the types it is called on share (`new`, `root`, `write`), so a mention of that name does not show
/// who uses this file; a mention of the type does.
fn in_impl(facts: &FileFacts, def: usize) -> bool {
    facts.defs[def]
        .parent
        .is_some_and(|parent| facts.defs[parent].symbol.kind.name == "impl")
}

/// `label:first..last`, or `label:first` for one line.
pub fn span(label: &str, first: usize, last: usize) -> String {
    if first == last {
        format!("{label}:{first}")
    } else {
        format!("{label}:{first}..{last}")
    }
}
