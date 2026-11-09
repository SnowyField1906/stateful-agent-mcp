//! The part of a codebase that one feature lives in: a personalized walk from the seed
//! definitions over the mention graph, and the shortest way in from the outside.

use std::collections::{HashMap, VecDeque};

use super::codebase::Codebase;
use super::graph::Tag;

/// What flows to a definition that mentions the definition: dependents weigh less than
/// dependencies.
const DEPENDENT_WEIGHT: f64 = 0.8;
/// Members of one type or trait.
const SAME_OWNER_WEIGHT: f64 = 0.5;
/// Definitions of one file.
const SAME_FILE_WEIGHT: f64 = 0.2;
/// The share of the walk that returns to the seeds each round.
const RESTART: f64 = 0.3;
/// A group of siblings or a file with more definitions than this links nothing: it says too
/// little about any one of them.
const MAX_GROUP: usize = 24;

/// The links of every definition, as `(node, share)` with the shares adding up to one. Only
/// listed definitions, which other code names and which are not test code, take part.
fn links(codebase: &Codebase) -> Vec<Vec<(usize, f64)>> {
    let graph = &codebase.graph;
    let mut owners: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for node in 0..graph.len() {
        if !codebase.listed(node) {
            continue;
        }
        if let Some(parent) = codebase.def(node).parent {
            owners
                .entry((codebase.file_of(node), parent))
                .or_default()
                .push(node);
        }
    }
    (0..graph.len())
        .map(|node| {
            if !codebase.listed(node) {
                return Vec::new();
            }
            let mut row: HashMap<usize, f64> = HashMap::new();
            for edge in graph.uses(node) {
                if edge.to != node && codebase.listed(edge.to) {
                    *row.entry(edge.to).or_default() += edge.weight;
                }
            }
            for edge in graph.used_by(node) {
                if edge.to != node && codebase.listed(edge.to) {
                    *row.entry(edge.to).or_default() += edge.weight * DEPENDENT_WEIGHT;
                }
            }
            let file = codebase.file_of(node);
            if let Some(parent) = codebase.def(node).parent {
                if let Some(group) = owners.get(&(file, parent)) {
                    if group.len() <= MAX_GROUP {
                        for &other in group.iter().filter(|&&other| other != node) {
                            *row.entry(other).or_default() += SAME_OWNER_WEIGHT;
                        }
                    }
                }
            }
            let in_file = graph.nodes_of(file);
            if in_file.len() <= MAX_GROUP * 2 {
                for other in in_file.filter(|&other| other != node && codebase.listed(other)) {
                    *row.entry(other).or_default() += SAME_FILE_WEIGHT;
                }
            }
            let mut row: Vec<(usize, f64)> = row.into_iter().collect();
            row.sort_by_key(|&(other, _)| other);
            let total: f64 = row.iter().map(|&(_, w)| w).sum();
            if total > 0.0 {
                for (_, w) in &mut row {
                    *w /= total;
                }
            }
            row
        })
        .collect()
}

/// The weight of every definition after `rings` rounds of the walk from `seeds`, by node. Each
/// round a definition passes its weight to its links and the seeds get 30% of the start back.
pub fn diffuse(codebase: &Codebase, seeds: &[usize], rings: usize) -> Vec<f64> {
    let mut start = vec![0.0; codebase.graph.len()];
    if seeds.is_empty() {
        return start;
    }
    let share = 1.0 / seeds.len() as f64;
    for &seed in seeds {
        start[seed] += share;
    }
    let links = links(codebase);
    let mut now = start.clone();
    for _ in 0..rings.max(1) {
        let mut next: Vec<f64> = start.iter().map(|p| p * RESTART).collect();
        for (node, &mass) in now.iter().enumerate() {
            if mass <= 0.0 {
                continue;
            }
            for &(to, share) in &links[node] {
                next[to] += (1.0 - RESTART) * mass * share;
            }
        }
        now = next;
    }
    now
}

/// A chain of callers from a seed up to where execution enters.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    /// The seed first, the outermost caller last.
    pub nodes: Vec<usize>,
    /// The last one is a door: named from outside and mentioned by nothing.
    pub door: bool,
}

/// The shortest chain of dependents from any seed to a door, within `max_hops`. Without a door in
/// reach, the longest chain that ends where nothing listed mentions the definition. `None` when
/// no seed has a dependent and none is a door.
pub fn way_in(codebase: &Codebase, seeds: &[usize], max_hops: usize) -> Option<Chain> {
    let mut parent: HashMap<usize, usize> = HashMap::new();
    let mut depth: HashMap<usize, usize> = HashMap::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    for &seed in seeds {
        if depth.insert(seed, 0).is_none() {
            queue.push_back(seed);
        }
    }
    let mut far: Option<usize> = None;
    while let Some(node) = queue.pop_front() {
        let here = depth[&node];
        if codebase.def_tags[node] == Tag::Door {
            return Some(chain(&parent, node, true));
        }
        if far.is_none_or(|f| here > depth[&f]) {
            far = Some(node);
        }
        if here >= max_hops {
            continue;
        }
        let mut callers: Vec<_> = codebase
            .graph
            .used_by(node)
            .iter()
            .filter(|edge| codebase.listed(edge.to))
            .collect();
        callers.sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.to.cmp(&b.to)));
        for edge in callers {
            if !depth.contains_key(&edge.to) {
                depth.insert(edge.to, here + 1);
                parent.insert(edge.to, node);
                queue.push_back(edge.to);
            }
        }
    }
    let far = far?;
    (depth[&far] > 0).then(|| chain(&parent, far, false))
}

fn chain(parent: &HashMap<usize, usize>, end: usize, door: bool) -> Chain {
    let mut nodes = vec![end];
    let mut at = end;
    while let Some(&before) = parent.get(&at) {
        nodes.push(before);
        at = before;
    }
    nodes.reverse();
    Chain { nodes, door }
}
