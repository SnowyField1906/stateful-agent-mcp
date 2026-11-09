//! The layers of a codebase: which part rests on which.
//!
//! A codebase is cut into units: the folders that hold a manifest (a crate, a package) and, where
//! there is only one of those, the folders below it, opened until there is more than one. The
//! mentions between the files of two units add up to an edge between them. Units that rest on each
//! other form one strongly connected component (Tarjan) and share a layer. The layer of a unit is
//! the length of the longest chain of dependencies below it, so what rests on nothing is layer 0.
//! Inside a cycle, an edge from the lower unit to the higher one is printed as against the layers.
//!
//! A Rust workspace's manifests are a cross-check: a declared dependency with no mention between
//! the two units is printed as `declared, unused`, and a mention between two units that no declared
//! path joins in that direction (a crate cannot use one it does not depend on, directly or not) is
//! noise and is left out.
//!
//! What the line leaves out: test code and the units that hold only test code, the folders above
//! the manifests (they are no part of the product), and a mention backed by fewer than two distinct
//! names, because one name in common is more often a collision than a dependency.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::language::registry;

/// A mention between two units lighter than this is not a dependency of the layer line.
const MIN_EDGE: f64 = 0.5;
/// The mentions between two units come from at least this many distinct names.
const MIN_NAMES: usize = 2;
/// The most edges against the layers that the layer line prints.
const MAX_AGAINST: usize = 3;
/// The widest row of the layer line.
const ROW_WIDTH: usize = 120;

/// What the survey says about the mentions between files, as [`Layers::compute`] reads it.
pub struct Mentions<'a> {
    /// The weight of the mentions from each file to each other file.
    pub weights: &'a BTreeMap<(usize, usize), f64>,
    /// The distinct names behind those mentions.
    pub names: &'a BTreeMap<(usize, usize), BTreeSet<String>>,
    /// Whether each file is test code, by file number.
    pub tests: &'a [bool],
}

/// A part of the code that the layer line names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    /// The folder below the scan root, `/`-separated; empty for the root itself.
    pub dir: String,
    /// The last folder of `dir`, or more of it when two units would share the name.
    pub name: String,
    /// The unit is no part of the product: only test code, or a folder above the manifests.
    pub loose: bool,
}

#[derive(Debug)]
pub struct Layers {
    pub units: Vec<Unit>,
    /// The unit of each file, by file number.
    pub of_file: Vec<usize>,
    /// The layer of each unit: 0 for those that rest on nothing.
    pub layer: Vec<usize>,
    /// The weight of the mentions from each unit to each other unit.
    pub edges: BTreeMap<(usize, usize), f64>,
    /// Edges inside a cycle that go from the lower unit to the higher one, as (from, to).
    pub against: Vec<(usize, usize)>,
    /// Dependencies that a manifest declares and no mention uses, as (from, to).
    pub unused: Vec<(usize, usize)>,
    /// The number of units in the largest cycle.
    pub cycle: usize,
}

impl Layers {
    /// The layers of the files named `labels` (relative to `root`), given the mentions between
    /// them and the dependencies the manifests declare between folders
    /// (`Packages::member_links`).
    pub fn compute(
        labels: &[&str],
        mentions: &Mentions<'_>,
        root: &Path,
        links: &[(PathBuf, PathBuf)],
    ) -> Self {
        let is_test = |file: usize| mentions.tests.get(file).copied().unwrap_or(false);
        let (dirs, of_file, manifest) = split_units(labels, root);
        let count = dirs.len();
        // A folder above the manifests is no part of the product, and neither is a unit that
        // holds only test code.
        let any_manifest = manifest.iter().any(|&has| has);
        let mut loose: Vec<bool> = manifest.iter().map(|&has| any_manifest && !has).collect();
        let mut has_code = vec![false; count];
        for (file, &unit) in of_file.iter().enumerate() {
            has_code[unit] |= !is_test(file);
        }
        for (unit, code) in has_code.into_iter().enumerate() {
            loose[unit] |= !code;
        }
        let names = short_names(&dirs);
        let units: Vec<Unit> = dirs
            .into_iter()
            .zip(names)
            .zip(loose)
            .map(|((dir, name), loose)| Unit { dir, name, loose })
            .collect();

        let mut edges: BTreeMap<(usize, usize), f64> = BTreeMap::new();
        let mut behind: BTreeMap<(usize, usize), BTreeSet<&str>> = BTreeMap::new();
        for (&(from, to), &weight) in mentions.weights {
            if is_test(from) || is_test(to) {
                continue;
            }
            let (a, b) = (of_file[from], of_file[to]);
            if a == b || units[a].loose || units[b].loose {
                continue;
            }
            *edges.entry((a, b)).or_default() += weight;
            if let Some(names) = mentions.names.get(&(from, to)) {
                behind
                    .entry((a, b))
                    .or_default()
                    .extend(names.iter().map(String::as_str));
            }
        }

        let folders: Vec<PathBuf> = units.iter().map(|unit| root.join(&unit.dir)).collect();
        let find = |dir: &Path| folders.iter().position(|folder| folder.as_path() == dir);
        let declared = declared_paths(links, count, &find);
        // A crate cannot mention one that it does not depend on, directly or not: where the
        // manifests list the members, a mention against or beside every declared path is a name
        // in common, not a dependency.
        let kept: BTreeMap<(usize, usize), f64> = edges
            .iter()
            .filter(|&(&(a, b), &weight)| {
                weight >= MIN_EDGE
                    && behind
                        .get(&(a, b))
                        .is_some_and(|names| names.len() >= MIN_NAMES)
                    && (links.is_empty() || !(manifest[a] && manifest[b]) || declared[a][b])
            })
            .map(|(&pair, &weight)| (pair, weight))
            .collect();
        let mut adjacent: Vec<Vec<usize>> = vec![Vec::new(); count];
        for &(from, to) in kept.keys() {
            adjacent[from].push(to);
        }

        // Components come out dependencies first, so the layer of a component is known from the
        // ones before it.
        let comps = components(&adjacent);
        let cycle = comps.iter().map(Vec::len).max().unwrap_or(1);
        let mut comp_of = vec![0usize; count];
        for (at, members) in comps.iter().enumerate() {
            for &member in members {
                comp_of[member] = at;
            }
        }
        let mut comp_layer = vec![0usize; comps.len()];
        for (at, members) in comps.iter().enumerate() {
            let mut layer = 0;
            for &member in members {
                for &next in &adjacent[member] {
                    let other = comp_of[next];
                    if other != at {
                        layer = layer.max(comp_layer[other] + 1);
                    }
                }
            }
            comp_layer[at] = layer;
        }
        let layer: Vec<usize> = (0..count).map(|unit| comp_layer[comp_of[unit]]).collect();

        let weight = |from: usize, to: usize| kept.get(&(from, to)).copied().unwrap_or(0.0);
        let mut against: Vec<(usize, usize)> = Vec::new();
        for members in comps.iter().filter(|members| members.len() > 1) {
            // The unit that leans most on its peers is the highest.
            let score = |unit: usize| -> f64 {
                members
                    .iter()
                    .filter(|&&peer| peer != unit)
                    .map(|&peer| weight(unit, peer) - weight(peer, unit))
                    .sum()
            };
            for &a in members {
                for &b in members {
                    if a == b || weight(a, b) == 0.0 {
                        continue;
                    }
                    let (high_a, high_b) = (score(a), score(b));
                    let level = (high_a - high_b).abs() <= 1e-9;
                    if high_a < high_b - 1e-9 || (level && a > b) {
                        against.push((a, b));
                    }
                }
            }
        }
        against.sort_unstable();
        against.dedup();

        let mut unused: Vec<(usize, usize)> = Vec::new();
        for (from, to) in links {
            if let (Some(a), Some(b)) = (find(from), find(to))
                && a != b
                && !units[a].loose
                && !units[b].loose
                && !edges.contains_key(&(a, b))
            {
                unused.push((a, b));
            }
        }
        unused.sort_unstable();
        unused.dedup();

        Self {
            units,
            of_file,
            layer,
            edges,
            against,
            unused,
            cycle,
        }
    }

    /// The rows of the `layers:` section: the layer line, and the manifest cross-check when it
    /// found something. Empty when the code is one unit. No row is wider than [`ROW_WIDTH`].
    pub fn rows(&self) -> Vec<String> {
        let live: Vec<usize> = (0..self.units.len())
            .filter(|&unit| !self.units[unit].loose)
            .collect();
        if live.len() < 2 {
            return Vec::new();
        }
        let mut by_layer: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
        for &unit in &live {
            by_layer
                .entry(self.layer[unit])
                .or_default()
                .push(self.units[unit].name.as_str());
        }
        let chain = by_layer
            .into_values()
            .map(|mut names| {
                names.sort_unstable();
                names.join(",")
            })
            .collect::<Vec<_>>()
            .join(" < ");
        // Most units rest on each other: there are no layers to print, and the chain would be
        // one cut list of names that the projects row already gives.
        let tangled = self.cycle > 2 && self.cycle * 2 > live.len();
        let mut suffix = String::new();
        if !tangled {
            for &(from, to) in self.against.iter().take(MAX_AGAINST) {
                suffix.push_str(&format!(
                    " !{}>{}",
                    self.units[from].name, self.units[to].name
                ));
            }
            if self.against.len() > MAX_AGAINST {
                suffix.push_str(&format!(" +{}", self.against.len() - MAX_AGAINST));
            }
        }
        // The edges against the layers are what the line is for: a long chain gives way to them.
        let width = ROW_WIDTH.saturating_sub(suffix.chars().count());
        let first = if tangled {
            format!("{} of {} units rest on each other", self.cycle, live.len())
        } else {
            cut(chain, width) + &suffix
        };
        let mut rows = vec![first];
        if !self.unused.is_empty() {
            let list: Vec<String> = self
                .unused
                .iter()
                .map(|&(from, to)| format!("{}>{}", self.units[from].name, self.units[to].name))
                .collect();
            rows.push(cut(
                format!("declared, unused: {}", list.join(" ")),
                ROW_WIDTH,
            ));
        }
        rows
    }
}

/// `row`, cut to `width` characters.
fn cut(row: String, width: usize) -> String {
    if row.chars().count() <= width {
        return row;
    }
    let mut short: String = row.chars().take(width.saturating_sub(1)).collect();
    short.push('…');
    short
}

/// Which unit the manifests say rests on which, through any number of units: `[a][b]` when `a`
/// declares a dependency path to `b`.
fn declared_paths(
    links: &[(PathBuf, PathBuf)],
    count: usize,
    find: &dyn Fn(&Path) -> Option<usize>,
) -> Vec<Vec<bool>> {
    let mut declared = vec![vec![false; count]; count];
    for (from, to) in links {
        if let (Some(a), Some(b)) = (find(from), find(to))
            && a != b
        {
            declared[a][b] = true;
        }
    }
    for via in 0..count {
        for from in 0..count {
            if !declared[from][via] {
                continue;
            }
            for to in 0..count {
                if declared[via][to] {
                    declared[from][to] = true;
                }
            }
        }
    }
    declared
}

/// The folder that holds `path`.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// The folder that holds every file, as deep as it goes.
fn common_dir(labels: &[&str]) -> String {
    let mut common: Option<Vec<&str>> = None;
    for label in labels {
        let dir: Vec<&str> = parent(label)
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        common = Some(match common {
            None => dir,
            Some(shared) => shared
                .iter()
                .zip(&dir)
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| *a)
                .collect(),
        });
    }
    common.unwrap_or_default().join("/")
}

/// The nearest folder at or above the file `label` that holds a manifest of its language. `cache`
/// remembers which folders do.
fn home(
    label: &str,
    root: &Path,
    cache: &mut HashMap<(String, &'static str), bool>,
) -> Option<String> {
    let syntax = registry::for_path(label)?;
    let language = syntax.language().name;
    let manifests = syntax.manifests();
    let mut folder = parent(label);
    loop {
        let key = (folder.to_string(), language);
        let has = *cache.entry(key).or_insert_with(|| {
            manifests
                .iter()
                .any(|name| root.join(folder).join(name).is_file())
        });
        if has {
            return Some(folder.to_string());
        }
        if folder.is_empty() {
            return None;
        }
        folder = parent(folder);
    }
}

/// The folder of each unit, the unit of each file and whether the unit holds a manifest (a scan
/// with no manifest at all is opened by folders, and those count).
fn split_units(labels: &[&str], root: &Path) -> (Vec<String>, Vec<usize>, Vec<bool>) {
    let mut cache = HashMap::new();
    let floor = common_dir(labels);
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut homes: BTreeSet<String> = BTreeSet::new();
    for (file, label) in labels.iter().enumerate() {
        let unit_dir = match home(label, root, &mut cache) {
            Some(dir) => {
                homes.insert(dir.clone());
                dir
            }
            None => floor.clone(),
        };
        groups.entry(unit_dir).or_default().push(file);
    }
    let mut opened = false;
    if groups.len() == 1
        && let Some((base, files)) = groups.pop_first()
    {
        groups = descend(labels, base, files);
        opened = true;
    }
    let mut dirs = Vec::new();
    let mut manifest = Vec::new();
    let mut of_file = vec![0usize; labels.len()];
    for (unit, (dir, files)) in groups.into_iter().enumerate() {
        for file in files {
            of_file[file] = unit;
        }
        manifest.push(opened || homes.contains(&dir));
        dirs.push(dir);
    }
    (dirs, of_file, manifest)
}

/// The parts below `base` that hold `files`: one per folder, and one for the files written
/// directly in `base`. A part that is the only one and is a folder is opened in turn.
fn descend(labels: &[&str], mut base: String, files: Vec<usize>) -> BTreeMap<String, Vec<usize>> {
    loop {
        let mut parts: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for &file in &files {
            let label = labels[file];
            let rest = label
                .strip_prefix(base.as_str())
                .unwrap_or(label)
                .trim_start_matches('/');
            let key = match rest.split_once('/') {
                Some((first, _)) if base.is_empty() => first.to_string(),
                Some((first, _)) => format!("{base}/{first}"),
                None => base.clone(),
            };
            parts.entry(key).or_default().push(file);
        }
        let opens = parts.len() == 1 && parts.keys().next().is_some_and(|key| *key != base);
        if !opens {
            return parts;
        }
        base = parts.into_keys().next().unwrap_or_default();
    }
}

/// The shortest name of each folder that no other folder shares: its last segment, or more.
fn short_names(dirs: &[String]) -> Vec<String> {
    let segments: Vec<Vec<&str>> = dirs
        .iter()
        .map(|dir| dir.split('/').filter(|part| !part.is_empty()).collect())
        .collect();
    segments
        .iter()
        .map(|own| {
            if own.is_empty() {
                return ".".to_string();
            }
            for take in 1..=own.len() {
                let tail = &own[own.len() - take..];
                let clash = segments
                    .iter()
                    .filter(|other| other.len() >= take && other[other.len() - take..] == *tail)
                    .count();
                if clash == 1 {
                    return tail.join("/");
                }
            }
            own.join("/")
        })
        .collect()
}

/// Strongly connected components of the graph `adjacent`, a component after every component it
/// has an edge to.
fn components(adjacent: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let count = adjacent.len();
    let mut state = Tarjan {
        adjacent,
        order: vec![None; count],
        low: vec![0; count],
        open: vec![false; count],
        stack: Vec::new(),
        next: 0,
        found: Vec::new(),
    };
    for node in 0..count {
        if state.order[node].is_none() {
            state.visit(node);
        }
    }
    state.found
}

struct Tarjan<'a> {
    adjacent: &'a [Vec<usize>],
    order: Vec<Option<usize>>,
    low: Vec<usize>,
    open: Vec<bool>,
    stack: Vec<usize>,
    next: usize,
    found: Vec<Vec<usize>>,
}

impl Tarjan<'_> {
    fn visit(&mut self, node: usize) {
        self.order[node] = Some(self.next);
        self.low[node] = self.next;
        self.next += 1;
        self.stack.push(node);
        self.open[node] = true;
        let adjacent = self.adjacent;
        for &other in &adjacent[node] {
            match self.order[other] {
                None => {
                    self.visit(other);
                    self.low[node] = self.low[node].min(self.low[other]);
                }
                Some(seen) if self.open[other] => {
                    self.low[node] = self.low[node].min(seen);
                }
                Some(_) => {}
            }
        }
        if Some(self.low[node]) == self.order[node] {
            let mut members = Vec::new();
            while let Some(member) = self.stack.pop() {
                self.open[member] = false;
                members.push(member);
                if member == node {
                    break;
                }
            }
            members.sort_unstable();
            self.found.push(members);
        }
    }
}
