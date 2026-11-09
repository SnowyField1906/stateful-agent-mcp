//! `langtool_summarize_feature`: the code that implements a feature, seen as a slice of the mention
//! graph: the seeds, the core around them, the types they work on, what they use from outside the
//! slice, the way in from an entry point, the tests and a plan of line ranges to read.
//!
//! Token safety: the words and names of the input are capped, every section is packed into at most
//! `zoom.rows() / 12` rows of one line each, a read range is clipped to [`MAX_READ_SPAN`] lines, and
//! the whole body is cut at `zoom.rows()` rows.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;

use super::summarize_symbol::{candidates, labelled, push_note, qualified, split_query};
use crate::common::params::Names;
use crate::common::{AgentError, Result, ServerConfig, Workspace, root_of};
use crate::engine::lens::{
    codebase::Codebase,
    concept::{lookup, query_words},
    graph::in_test,
    slice::{diffuse, way_in},
    store::{FactStore, survey},
    zoom::{ROW, Zoom, cut, limit_lines},
};
use crate::engine::origins::Origins;
use crate::engine::scan::ScanTarget;

pub const DESCRIPTION: &str = "\
The code that implements a feature: seeds from `query` words or `symbol` names, the slice around \
them (core, data types, edges to outside code), the way in from an entry point, tests and a read \
plan with line ranges. `implicit_path` narrows the scan; `zoom` (`low`/`medium`/`high`/`max`) grows \
the rows. Use it before `langtool_read_file` when the task names a behaviour, not a definition.\
";

/// The longest query read, in characters.
const MAX_QUERY: usize = 200;
/// The most words of a query that take part.
const MAX_WORDS: usize = 12;
/// The most names that take part.
const MAX_NAMES: usize = 6;
/// The most lines of one definition that the read plan sends the agent to.
const MAX_READ_SPAN: usize = 80;
/// Ranges of one file closer than this many lines are read as one.
const MERGE_GAP: usize = 3;

/// Parameters for `langtool_summarize_feature`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SummarizeFeatureParams {
    /// Words that describe the feature, such as `patch hunk conflict`.
    #[serde(default)]
    pub query: Option<String>,
    /// One or more definition names (`name`, `Type::name`) to start from; they win over `query`.
    #[serde(default)]
    #[schemars(with = "Option<Vec<String>>")]
    pub symbol: Option<Names>,
    /// File or directory to scan, relative to the project root or absolute. Default `.`.
    #[serde(default)]
    pub implicit_path: Option<String>,
    /// How much to show: `low`, `medium` (default), `high` or `max`.
    #[serde(default)]
    pub zoom: Option<String>,
    /// Include dotfiles and gitignored files.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
}

pub fn summarize_feature(
    workspace: Option<&dyn Workspace>,
    cfg: &ServerConfig,
    store: &FactStore,
    p: SummarizeFeatureParams,
) -> Result<String> {
    let names: Vec<String> = p
        .symbol
        .as_ref()
        .map(|n| {
            n.cleaned()
                .into_iter()
                .take(MAX_NAMES)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut text: Option<String> = p
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(|q| cut(q, MAX_QUERY));
    if names.is_empty() && text.is_none() {
        return Err(AgentError::invalid(
            "query",
            "give `query` words or `symbol` names",
        ));
    }
    let (zoom, zoom_note) = Zoom::from_param(p.zoom.as_deref());
    let mut notes: Vec<String> = Vec::new();
    notes.extend(zoom_note);

    let target = ScanTarget {
        path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };
    let root = root_of(workspace);
    let sv = survey(workspace, &target, store)?;
    if sv.files.is_empty() {
        return Ok(finish("no supported files found\n".to_string(), &notes));
    }
    let origins = root
        .map(|r| Origins::new(r, &[]))
        .unwrap_or_else(|| Origins::new(Path::new(""), &[]));
    let codebase = Codebase::new(sv.files, &origins);

    // Wrong-tool tolerance: a lone word that names a definition is read as a symbol.
    let mut by_name = names.clone();
    if by_name.is_empty() {
        let single = text
            .as_deref()
            .filter(|q| !q.contains(char::is_whitespace))
            .map(str::to_string);
        if let Some(q) = single {
            let (name, qualifiers) = split_query(&q);
            if !candidates(&codebase, &name, &qualifiers).is_empty() {
                notes.push(format!("note: `{q}` names a definition; read as symbol"));
                by_name.push(q);
                text = None;
            }
        }
    }

    // Seeds from names.
    let mut seeds: Vec<usize> = Vec::new();
    let mut extra_words: Vec<String> = Vec::new();
    for name in &by_name {
        let (bare, qualifiers) = split_query(name);
        let mut found = candidates(&codebase, &bare, &qualifiers);
        if found.is_empty() {
            notes.push(format!("note: no symbol `{name}`; read as feature words"));
            extra_words.extend(query_words(name));
            continue;
        }
        found.sort_by(|&a, &b| {
            codebase
                .listed(b)
                .cmp(&codebase.listed(a))
                .then(
                    codebase
                        .graph
                        .load(b)
                        .ca
                        .total_cmp(&codebase.graph.load(a).ca),
                )
                .then(a.cmp(&b))
        });
        if found.len() > 1 {
            notes.push(format!(
                "note: `{name}` is defined {} times; took {}",
                found.len(),
                codebase.locator(found[0])
            ));
        }
        if !seeds.contains(&found[0]) {
            seeds.push(found[0]);
        }
    }

    // Seeds from words, when no name gave one.
    let label: String;
    if seeds.is_empty() {
        let mut words = text.as_deref().map(query_words).unwrap_or_default();
        for word in extra_words {
            if !words.contains(&word) {
                words.push(word);
            }
        }
        words.truncate(MAX_WORDS);
        if words.is_empty() {
            return Ok(finish(
                "no usable words: words need three letters or more\n".to_string(),
                &notes,
            ));
        }
        label = words.join(" ");
        seeds = lookup(&codebase, &words, 2 + 2 * zoom.rings())
            .into_iter()
            .map(|hit| hit.node)
            .collect();
        if seeds.is_empty() {
            let out = format!(
                "no definition matches \"{}\"\nhint: > langtool_find_symbols query=[\"…\"] searches names by substring\n",
                cut(&label, 60)
            );
            return Ok(finish(out, &notes));
        }
    } else {
        label = by_name.join(", ");
    }

    // The slice.
    let rows_cap = (zoom.rows() / 12).max(2);
    let slice_limit = zoom.rows() / 4;
    let weight = diffuse(&codebase, &seeds, zoom.rings());
    let seed_set: BTreeSet<usize> = seeds.iter().copied().collect();
    let mut core: Vec<usize> = (0..weight.len())
        .filter(|n| weight[*n] > 0.0 && codebase.listed(*n) && !seed_set.contains(n))
        .collect();
    core.sort_by(|&a, &b| weight[b].total_cmp(&weight[a]).then(a.cmp(&b)));
    core.truncate(slice_limit);
    let core_set: BTreeSet<usize> = core.iter().copied().collect();

    let files: BTreeSet<usize> = seeds
        .iter()
        .chain(core.iter())
        .map(|&n| codebase.file_of(n))
        .collect();
    let mut out = format!(
        "feature \"{}\" · {} seeds · slice {} defs in {} files · zoom {}\n",
        cut(&label, 60),
        seeds.len(),
        seeds.len() + core.len(),
        files.len(),
        zoom.name()
    );

    // Rows: seed, way in, core, data, edge, tests, read.
    let seed_items: Vec<String> = seeds.iter().map(|&n| short(&codebase, n)).collect();
    labelled(&mut out, "seed:", &pack(&seed_items, " · ", rows_cap));

    let chain = way_in(&codebase, &seeds, zoom.rings() + 3);
    if let Some(chain) = &chain {
        let mut items: Vec<String> = chain.nodes.iter().map(|&n| short(&codebase, n)).collect();
        if chain.door {
            items.push("(entry)".to_string());
        }
        labelled(&mut out, "way in:", &pack(&items, " ← ", 3));
    }

    let core_items: Vec<String> = core.iter().map(|&n| short(&codebase, n)).collect();
    labelled(&mut out, "core:", &pack(&core_items, " · ", rows_cap));

    let mut data_w: HashMap<usize, f64> = HashMap::new();
    let mut edge_w: HashMap<usize, f64> = HashMap::new();
    for &n in seeds.iter().chain(core.iter()) {
        for edge in codebase.graph.uses(n) {
            let to = edge.to;
            if to == n || seed_set.contains(&to) || !codebase.listed(to) {
                continue;
            }
            if codebase.def(to).symbol.kind.declares_type {
                *data_w.entry(to).or_default() += edge.weight;
            } else if !core_set.contains(&to) {
                *edge_w.entry(to).or_default() += edge.weight;
            }
        }
    }
    let data_items: Vec<String> = one_per_name(&codebase, heaviest(data_w))
        .into_iter()
        .map(|n| short(&codebase, n))
        .collect();
    labelled(&mut out, "data:", &pack(&data_items, " · ", rows_cap));
    // What the slice uses from outside it, without the accessors (`len`, `first`) and the names that
    // a tenth of the files use (`push`), which every slice calls and none is about; when nothing
    // else is left they stay.
    let mut edge_nodes = one_per_name(&codebase, heaviest(edge_w));
    let weighty: Vec<usize> = edge_nodes
        .iter()
        .copied()
        .filter(|&n| !codebase.is_accessor(n) && !codebase.is_common(n))
        .collect();
    if !weighty.is_empty() {
        edge_nodes = weighty;
    }
    let edge_items: Vec<String> = edge_nodes
        .into_iter()
        .map(|n| short(&codebase, n))
        .collect();
    labelled(&mut out, "edge:", &pack(&edge_items, " · ", rows_cap));

    let mut tests: BTreeMap<usize, usize> = BTreeMap::new();
    for &seed in &seeds {
        for edge in codebase.graph.used_by(seed) {
            if in_test(codebase.def(edge.to)) {
                *tests.entry(codebase.file_of(edge.to)).or_default() += 1;
            }
        }
    }
    let mut tests: Vec<(usize, usize)> = tests.into_iter().collect();
    tests.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let test_items: Vec<String> = tests
        .iter()
        .map(|&(file, count)| format!("{} ({count} mentions)", codebase.label(file)))
        .collect();
    labelled(&mut out, "tests:", &pack(&test_items, " · ", rows_cap));

    let mut plan: Vec<usize> = seeds.clone();
    if let Some(chain) = &chain {
        plan.extend(chain.nodes.iter().copied());
    }
    plan.extend(core.iter().take(rows_cap * 2).copied());
    labelled(&mut out, "read:", &read_plan(&codebase, &plan, rows_cap));

    let mut out = limit_lines(&out, zoom.rows());
    if let Some(next) = zoom.next() {
        notes.push(format!("hint: use zoom={} for more detail", next.name()));
    }
    out = finish(out, &notes);
    Ok(out)
}

/// `out` with the notes, at most four, as its last lines.
fn finish(mut out: String, notes: &[String]) -> String {
    for note in notes.iter().take(4) {
        push_note(&mut out, Some(note.clone()));
    }
    out
}

/// `Name file.rs:line`, the short form that one row of a feature holds many of.
fn short(codebase: &Codebase, node: usize) -> String {
    let label = codebase.label(codebase.file_of(node));
    format!(
        "{} {}:{}",
        qualified(codebase, node),
        label.rsplit('/').next().unwrap_or(label),
        codebase.def(node).symbol.head
    )
}

/// `nodes` without a second definition of the same name in the same file (a type and its impl, two
/// overloads), which a row of names would print twice.
fn one_per_name(codebase: &Codebase, nodes: Vec<usize>) -> Vec<usize> {
    let mut seen: BTreeSet<(usize, String)> = BTreeSet::new();
    nodes
        .into_iter()
        .filter(|&node| {
            let key = (
                codebase.file_of(node),
                codebase.def(node).symbol.name.clone(),
            );
            seen.insert(key)
        })
        .collect()
}

/// The nodes of `weights`, the heaviest first, ties by node.
fn heaviest(weights: HashMap<usize, f64>) -> Vec<usize> {
    let mut rows: Vec<(usize, f64)> = weights.into_iter().collect();
    rows.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    rows.into_iter().map(|(node, _)| node).collect()
}

/// `items` joined with `sep` into rows that fit one line, at most `max_rows` of them; what does not
/// fit is counted at the end of the last row.
fn pack(items: &[String], sep: &str, max_rows: usize) -> Vec<String> {
    let width = ROW - 8;
    let sep_len = sep.chars().count();
    let mut rows: Vec<String> = Vec::new();
    let mut used = 0usize;
    for item in items {
        let item = cut(item, width);
        let joined = match rows.last_mut() {
            Some(row) if row.chars().count() + sep_len + item.chars().count() <= width => {
                row.push_str(sep);
                row.push_str(&item);
                true
            }
            _ => false,
        };
        if !joined {
            if rows.len() >= max_rows {
                break;
            }
            rows.push(item);
        }
        used += 1;
    }
    if used < items.len() {
        if let Some(row) = rows.last_mut() {
            row.push_str(&format!(" … +{}", items.len() - used));
        }
    }
    rows
}

/// The line ranges to read, per file: source before test code, then the files that the others in
/// the plan use before the ones that use them (a callee is read before its callers), then the order
/// the nodes name them. Each definition is clipped to [`MAX_READ_SPAN`] lines, close ranges merged.
fn read_plan(codebase: &Codebase, nodes: &[usize], rows_cap: usize) -> Vec<String> {
    let mut order: Vec<usize> = Vec::new();
    let mut spans: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
    for &node in nodes {
        let file = codebase.file_of(node);
        let symbol = &codebase.def(node).symbol;
        let last = symbol.last.min(symbol.first + MAX_READ_SPAN - 1);
        if !spans.contains_key(&file) {
            order.push(file);
        }
        spans.entry(file).or_default().push((symbol.first, last));
    }
    let planned: BTreeSet<usize> = nodes.iter().copied().collect();
    let mut called: HashMap<usize, f64> = HashMap::new();
    for &node in nodes {
        let file = codebase.file_of(node);
        for edge in codebase.graph.uses(node) {
            let callee = codebase.file_of(edge.to);
            if callee != file && planned.contains(&edge.to) {
                *called.entry(callee).or_default() += edge.weight;
            }
        }
    }
    let named: HashMap<usize, usize> = order.iter().enumerate().map(|(i, &f)| (f, i)).collect();
    let called_by_plan = |file: usize| called.get(&file).copied().unwrap_or(0.0);
    order.sort_by(|&a, &b| {
        codebase
            .is_test_file(a)
            .cmp(&codebase.is_test_file(b))
            .then(called_by_plan(b).total_cmp(&called_by_plan(a)))
            .then(named[&a].cmp(&named[&b]))
    });
    let items: Vec<String> = order
        .iter()
        .map(|file| {
            let mut ranges = spans[file].clone();
            ranges.sort();
            let mut merged: Vec<(usize, usize)> = Vec::new();
            for (first, last) in ranges {
                if let Some(prev) = merged.last_mut() {
                    if first <= prev.1 + MERGE_GAP {
                        prev.1 = prev.1.max(last);
                        continue;
                    }
                }
                merged.push((first, last));
            }
            let parts: Vec<String> = merged
                .iter()
                .map(|&(a, b)| {
                    if a == b {
                        a.to_string()
                    } else {
                        format!("{a}..{b}")
                    }
                })
                .collect();
            format!("{} {}", codebase.label(*file), parts.join(", "))
        })
        .collect();
    pack(&items, " · ", rows_cap)
}
