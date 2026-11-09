//! `langtool_summarize_symbol`: one definition seen through the mention graph: what it is, what it
//! uses, what uses it, its kin and where the work last touched it.

use std::collections::BTreeSet;
use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;

use super::summarize_feature::{SummarizeFeatureParams, summarize_feature};
use super::summarize_symbol_story::{call_sites, source_of, story_skeleton};
use crate::common::{AgentError, ItemKind, Result, ServerConfig, Workspace, root_of};
use crate::engine::lens::{
    codebase::Codebase,
    graph::{Edge, Tag, in_test, is_named},
    store::{FactStore, survey},
    words::{Words, subwords},
    zoom::{ROW, Zoom, cut, fit, limit_lines},
};
use crate::engine::origins::Origins;
use crate::engine::scan::ScanTarget;
use crate::engine::scope::bare_name;

pub const DESCRIPTION: &str = "\
One definition seen through the mention graph: where it is, its signature and doc, its role (base, \
hub, door or detail), the definitions it uses and the ones that use it, kin that share its name \
words and where the work last touched it. `symbol` is `name`, `Type::name` or `mod/Type/name`; \
`implicit_path` narrows the scan (default `.`). The zoom (`low`/`medium`/`high`/`max`, default \
`medium`) grows the rows per section; `high` adds the story, the body folded to its control flow with \
what each project call is, and the open work that names the symbol; `max` adds the call sites. Use it \
before reading a definition with `langtool_read_file`. A `symbol` of several words that no definition \
has is answered as `langtool_summarize_feature` with those words as `query`.\
";

/// The last line of that answer, so that the agent knows which tool takes what.
const FEATURE_HINT: &str = "hint: `symbol` takes one definition name; words that describe a behaviour go in `query` of langtool_summarize_feature\n";

/// Parameters for `langtool_summarize_symbol`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SummarizeSymbolParams {
    /// Definition to summarize: `name`, `Type::name`, `Type.name` or `mod/Type/name`.
    pub symbol: String,
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

pub fn summarize_symbol(
    workspace: Option<&dyn Workspace>,
    cfg: &ServerConfig,
    store: &FactStore,
    p: SummarizeSymbolParams,
) -> Result<String> {
    let query = p.symbol.trim();
    if query.is_empty() {
        return Err(AgentError::invalid("symbol", "must not be empty"));
    }
    let (zoom, zoom_note) = Zoom::from_param(p.zoom.as_deref());
    let target = ScanTarget {
        path: p.implicit_path.as_deref(),
        max_depth: cfg.max_dir_depth,
        show_gitignored: p.show_gitignored,
    };

    let root = root_of(workspace);
    let sv = survey(workspace, &target, store)?;
    let mut out = String::new();
    if sv.files.is_empty() {
        out.push_str("no supported files found\n");
        push_note(&mut out, zoom_note);
        return Ok(out);
    }
    let origins = root
        .map(|r| Origins::new(r, &[]))
        .unwrap_or_else(|| Origins::new(Path::new(""), &[]));
    let codebase = Codebase::new(sv.files, &origins);

    let (name, qualifiers) = split_query(query);
    let mut found = candidates(&codebase, &name, &qualifiers);
    if found.is_empty() {
        if query.split_whitespace().nth(1).is_some() {
            // Several words name no definition: they describe a feature, so answer as that.
            let mut answer = summarize_feature(
                workspace,
                cfg,
                store,
                SummarizeFeatureParams {
                    query: Some(query.to_string()),
                    symbol: None,
                    implicit_path: p.implicit_path.clone(),
                    zoom: p.zoom.clone(),
                    show_gitignored: p.show_gitignored,
                },
            )?;
            if !answer.ends_with('\n') {
                answer.push('\n');
            }
            answer.push_str(FEATURE_HINT);
            return Ok(format!(
                "no definition named `{query}`: the words are read as `query` of langtool_summarize_feature\n{answer}"
            ));
        }
        out.push_str(&not_found(&codebase, query, &name));
        push_note(&mut out, zoom_note);
        return Ok(out);
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

    let node = found[0];
    let def = codebase.def(node);
    let file = codebase.file_of(node);
    let label = codebase.label(file);
    let cap = (zoom.rows() / 6).max(4);
    let mut partial = false;

    // 1. Head, signature and doc.
    let tag = codebase.def_tags[node].as_str();
    let mut tail = format!("  {}  {}", def.symbol.kind.name, codebase.locator(node));
    if !tag.is_empty() {
        tail.push_str(&format!("  {tag}"));
    }
    if in_test(def) {
        tail.push_str("  test");
    }
    tail.push_str(&format!("  zoom {}", zoom.name()));
    // A long row loses the end of the name, never the line range that the reader needs.
    let room = ROW.saturating_sub(tail.chars().count()).max(16);
    out.push_str(&fit(&format!(
        "{}{tail}",
        cut(&qualified(&codebase, node), room)
    )));
    out.push('\n');
    labelled(&mut out, "sig:", &[def.symbol.signature.clone()]);
    if !def.symbol.doc.is_empty() {
        labelled(&mut out, "doc:", &[def.symbol.doc.clone()]);
    }
    if zoom >= Zoom::Medium {
        if let Some(doc) = &codebase.facts(file).doc {
            labelled(&mut out, "file:", &[doc.clone()]);
        }
    }
    if found.len() > 1 {
        let rows: Vec<String> = found[1..]
            .iter()
            .take(cap)
            .map(|&n| {
                format!(
                    "{}  {}  {}",
                    qualified(&codebase, n),
                    codebase.def(n).symbol.kind.name,
                    codebase.locator(n)
                )
            })
            .collect();
        labelled(&mut out, "also:", &rows);
    }

    // 2. Role.
    labelled(&mut out, "role:", &[role_row(&codebase, node)]);

    // 3. Uses and used by.
    let uses = ranked(&codebase, node, codebase.graph.uses(node));
    let used_by = ranked(&codebase, node, codebase.graph.used_by(node));
    edge_section(&mut out, &codebase, "uses", &uses, cap, &mut partial);
    edge_section(&mut out, &codebase, "used by", &used_by, cap, &mut partial);
    if partial {
        out.push_str(
            "note: ~w marks a name defined in several places; w is the share of the mention\n",
        );
    }

    // 4. Kin.
    if zoom >= Zoom::Medium {
        let mut skip: BTreeSet<usize> = found.iter().copied().collect();
        skip.extend(uses.iter().map(|e| e.to));
        skip.extend(used_by.iter().map(|e| e.to));
        let per_file: Vec<Vec<String>> = (0..codebase.files.len())
            .map(|f| {
                codebase
                    .graph
                    .nodes_of(f)
                    .filter(|&n| codebase.listed(n))
                    .map(|n| codebase.def(n).symbol.name.clone())
                    .collect()
            })
            .collect();
        let words = Words::new(&per_file);
        let kin = kin(&codebase, &words, node, &skip);
        if !kin.is_empty() {
            let mut rows: Vec<String> = kin
                .iter()
                .take(cap)
                .map(|(n, shared)| {
                    format!(
                        "{}  {}  {}  shares {shared}",
                        qualified(&codebase, *n),
                        codebase.def(*n).symbol.kind.name,
                        codebase.locator(*n)
                    )
                })
                .collect();
            if kin.len() > cap {
                rows.push(format!("… {} more", kin.len() - cap));
            }
            labelled(&mut out, "kin:", &rows);
        }
    }

    // 5. Story from the source, then touch and the open work from the ledger.
    if zoom >= Zoom::High {
        if let Some(text) = source_of(workspace, label) {
            let rows = story_skeleton(&codebase, node, &text, (zoom.rows() / 3).max(8));
            if !rows.is_empty() {
                labelled(&mut out, "story:", &rows);
            }
        }
        if zoom >= Zoom::Max {
            let rows = call_sites(workspace, &codebase, node, &used_by, cap);
            if !rows.is_empty() {
                labelled(&mut out, "sites:", &rows);
            }
        }
    }
    if let Some(ws) = workspace {
        if zoom >= Zoom::Medium {
            let rows = touch_rows(ws, label);
            if !rows.is_empty() {
                labelled(&mut out, "touch:", &rows);
            }
        }
        if zoom >= Zoom::High {
            let rows = work_rows(ws, label, &def.symbol.name, cap);
            if !rows.is_empty() {
                labelled(&mut out, "work:", &rows);
            }
        }
    }

    // 6. Next step and hint.
    out.push_str(&fit(&format!(
        "next:   > langtool_read_file file_path={label} symbol={}",
        qualified(&codebase, node)
    )));
    out.push('\n');
    push_note(&mut out, zoom_note);
    if let Some(next) = zoom.next() {
        out.push_str(&format!("hint: use zoom={} for more detail\n", next.name()));
    }

    Ok(limit_lines(&out, zoom.rows() + 16))
}

// ── helpers ────────────────────────────────────────────────────────────────────

pub(super) fn push_note(out: &mut String, note: Option<String>) {
    if let Some(note) = note {
        out.push_str(&note);
        out.push('\n');
    }
}

/// The name and the qualifiers of a query: `Type::name`, `Type.name` and `mod/Type/name`.
pub(super) fn split_query(query: &str) -> (String, Vec<String>) {
    let mut parts: Vec<&str> = query
        .split(|c: char| matches!(c, ':' | '.' | '/'))
        .filter(|part| !part.is_empty())
        .collect();
    let name = parts.pop().unwrap_or(query);
    (
        bare_name(name).to_string(),
        parts
            .iter()
            .map(|part| bare_name(part).to_string())
            .collect(),
    )
}

/// The nodes of the definitions called `name` whose enclosing definitions end in `qualifiers`.
pub(super) fn candidates(codebase: &Codebase, name: &str, qualifiers: &[String]) -> Vec<usize> {
    (0..codebase.graph.len())
        .filter(|&node| {
            let def = codebase.def(node);
            if !is_named(def) || bare_name(&def.symbol.name) != name {
                return false;
            }
            let scope: Vec<&str> = def.symbol.scope.names().map(bare_name).collect();
            scope.len() >= qualifiers.len()
                && scope[scope.len() - qualifiers.len()..]
                    .iter()
                    .zip(qualifiers)
                    .all(|(have, want)| *have == want.as_str())
        })
        .collect()
}

/// The answer for a name that no definition has: the names that contain it.
fn not_found(codebase: &Codebase, query: &str, name: &str) -> String {
    let needle = name.to_lowercase();
    let mut similar: Vec<usize> = (0..codebase.graph.len())
        .filter(|&n| {
            codebase.listed(n) && codebase.def(n).symbol.name.to_lowercase().contains(&needle)
        })
        .collect();
    similar.sort_by(|&a, &b| {
        codebase
            .graph
            .load(b)
            .ca
            .total_cmp(&codebase.graph.load(a).ca)
            .then(a.cmp(&b))
    });
    let mut out = format!("no definition named `{query}`\n");
    if similar.is_empty() {
        out.push_str("hint: > langtool_find_symbols query=[\"…\"] searches names by substring\n");
        return out;
    }
    out.push_str("similar:\n");
    for &n in similar.iter().take(8) {
        out.push_str(&fit(&format!(
            "  {}  {}  {}",
            qualified(codebase, n),
            codebase.def(n).symbol.kind.name,
            codebase.locator(n)
        )));
        out.push('\n');
    }
    out
}

/// `Owner::name` for a member of a type or trait, otherwise the name.
pub(super) fn qualified(codebase: &Codebase, node: usize) -> String {
    let symbol = &codebase.def(node).symbol;
    match symbol.scope.owner() {
        Some(owner) => format!("{}::{}", owner.bare_name(), symbol.name),
        None => symbol.name.clone(),
    }
}

/// `label` padded to eight columns before the first row, the other rows indented under it.
pub(super) fn labelled(out: &mut String, label: &str, rows: &[String]) {
    for (i, row) in rows.iter().enumerate() {
        if i == 0 {
            out.push_str(&format!("{label:<8}"));
        } else {
            out.push_str("        ");
        }
        out.push_str(&fit(row));
        out.push('\n');
    }
}

fn role_row(codebase: &Codebase, node: usize) -> String {
    let load = codebase.graph.load(node);
    let (word, meaning) = match codebase.def_tags[node] {
        Tag::Base => (
            "base",
            "; many depend on it and it depends on little: read it first",
        ),
        Tag::Hub => (
            "hub",
            "; it depends on much and has dependents: read it to see the flow",
        ),
        Tag::Door => (
            "door",
            "; named from outside and mentioned by nothing: execution starts here",
        ),
        Tag::Detail => ("detail", ""),
    };
    let test = if in_test(codebase.def(node)) {
        " (test code)"
    } else {
        ""
    };
    format!(
        "{word}{test}  ca {:.1}  ce {:.1}{meaning}",
        load.ca, load.ce
    )
}

/// The edges without the one back to `node`, the heaviest first, then the most used.
fn ranked(codebase: &Codebase, node: usize, edges: &[Edge]) -> Vec<Edge> {
    let mut edges: Vec<Edge> = edges.iter().copied().filter(|e| e.to != node).collect();
    edges.sort_by(|a, b| {
        b.weight
            .total_cmp(&a.weight)
            .then(
                codebase
                    .graph
                    .load(b.to)
                    .ca
                    .total_cmp(&codebase.graph.load(a.to).ca),
            )
            .then(a.to.cmp(&b.to))
    });
    edges
}

fn row_of(codebase: &Codebase, edge: &Edge) -> String {
    let node = edge.to;
    let mut row = format!(
        "  {}  {}  {}",
        qualified(codebase, node),
        codebase.def(node).symbol.kind.name,
        codebase.locator(node)
    );
    let tag = codebase.def_tags[node].as_str();
    if !tag.is_empty() {
        row.push_str(&format!("  {tag}"));
    }
    if in_test(codebase.def(node)) {
        row.push_str("  test");
    }
    if edge.weight < 0.95 {
        row.push_str(&format!("  ~{:.1}", edge.weight));
    }
    fit(&row)
}

fn edge_section(
    out: &mut String,
    codebase: &Codebase,
    label: &str,
    edges: &[Edge],
    cap: usize,
    partial: &mut bool,
) {
    if edges.is_empty() {
        out.push_str(&format!("{label}: nothing in the survey\n"));
        return;
    }
    out.push_str(&format!("{label} ({}):\n", edges.len()));
    for edge in edges.iter().take(cap) {
        if edge.weight < 0.95 {
            *partial = true;
        }
        out.push_str(&row_of(codebase, edge));
        out.push('\n');
    }
    if edges.len() > cap {
        out.push_str(&format!("  … {} more\n", edges.len() - cap));
    }
}

/// Listed definitions outside `skip` that share a distinguishing word of the name of `node`, the
/// most used first, each with the word.
fn kin(
    codebase: &Codebase,
    words: &Words,
    node: usize,
    skip: &BTreeSet<usize>,
) -> Vec<(usize, String)> {
    let keys = words.top(&[codebase.def(node).symbol.name.clone()], 2);
    if keys.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(usize, String)> = (0..codebase.graph.len())
        .filter(|n| *n != node && !skip.contains(n) && codebase.listed(*n))
        .filter_map(|n| {
            let shared = subwords(&codebase.def(n).symbol.name)
                .into_iter()
                .find(|word| keys.contains(word))?;
            Some((n, shared))
        })
        .collect();
    out.sort_by(|(a, _), (b, _)| {
        codebase
            .graph
            .load(*b)
            .ca
            .total_cmp(&codebase.graph.load(*a).ca)
            .then(a.cmp(b))
    });
    out
}

/// The last turn that changed the file, and whether it is in the work front.
fn touch_rows(workspace: &dyn Workspace, label: &str) -> Vec<String> {
    let mut rows = Vec::new();
    if let Some(turn) = workspace.last_turn_touching(label) {
        rows.push(format!("file last changed in turn {turn}"));
    }
    if let Some(moved) = workspace
        .work_front()
        .moved
        .iter()
        .find(|moved| moved.path == label)
    {
        let who = if moved.outside {
            "outside the ledger"
        } else {
            "by you"
        };
        rows.push(format!(
            "in the work front: turn {} ({who}), {} turns ago",
            moved.turn, moved.age
        ));
    }
    rows
}

/// Open tasks and notes of the latest sessions that name the symbol or its file.
fn work_rows(workspace: &dyn Workspace, label: &str, name: &str, cap: usize) -> Vec<String> {
    let file_name = label.rsplit('/').next().unwrap_or(label);
    let specific = !matches!(file_name, "mod.rs" | "lib.rs" | "main.rs" | "__init__.py");
    workspace
        .work_front()
        .open
        .iter()
        .filter(|item| {
            mentions(&item.text, name)
                || item.text.contains(label)
                || (specific && item.text.contains(file_name))
        })
        .take(cap)
        .map(|item| {
            let kind = match item.kind {
                ItemKind::Task => "task",
                ItemKind::Note => "note",
            };
            format!("{kind} {}: {}", item.id, item.text)
        })
        .collect()
}

/// Whether `text` names `name` as a whole word.
fn mentions(text: &str, name: &str) -> bool {
    if name.len() < 3 {
        return false;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}
