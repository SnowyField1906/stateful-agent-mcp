//! The story of `langtool_summarize_symbol`: the body of a definition folded to its control flow,
//! each project call named by what its callee is, and the places that call the definition.

use std::collections::{BTreeSet, HashMap};

use crate::common::fs::resolve::Scope;
use crate::common::{Workspace, root_of};
use crate::engine::fold::{self, Row};
use crate::engine::lens::{
    codebase::Codebase,
    graph::{Edge, in_test},
    zoom::{cut, fit},
};
use crate::engine::references::Role;

/// The largest source file that the story reads, in bytes.
const MAX_STORY_BYTES: u64 = 4 << 20;

/// The text of the file `label`, as the survey names it. The agent is shown lines of it, so it is
/// registered as seen: a patch that quotes the story is then recognised as a quote of this version.
pub(super) fn source_of(workspace: Option<&dyn Workspace>, label: &str) -> Option<String> {
    let scope = Scope::new(root_of(workspace), Some(label), "file_path").ok()?;
    if std::fs::metadata(&scope.target.abs).ok()?.len() > MAX_STORY_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(&scope.target.abs).ok()?;
    if scope.in_project
        && let Some(workspace) = workspace
    {
        workspace.note_seen(&scope.target.rel, &text);
    }
    Some(text)
}

/// What a callee is, in at most 60 characters: its first documentation line, else its signature.
fn gist(codebase: &Codebase, node: usize) -> String {
    let symbol = &codebase.def(node).symbol;
    let text = if symbol.doc.is_empty() {
        symbol.signature.as_str()
    } else {
        symbol.doc.as_str()
    };
    cut(text, 60)
}

/// The body of a definition folded to its control flow, as rows of source lines. A row that calls a
/// project definition ends in what the callee is: `→ locate: Byte offset of the single intended
/// match`, with a `?` after the name when several definitions have it. The reader sees what the
/// function does, in order, without opening a callee.
pub(super) fn story_skeleton(
    codebase: &Codebase,
    node: usize,
    text: &str,
    cap: usize,
) -> Vec<String> {
    let id = codebase.graph.id(node);
    let facts = codebase.facts(id.file);
    let def = &facts.defs[id.def];
    // The blocks around the definition (an impl, a class) count as depth too.
    let mut depth = 1;
    let mut parent = def.parent;
    while let Some(at) = parent {
        depth += 1;
        parent = facts.defs[at].parent;
    }
    let (_, all) = fold::fold_file(codebase.label(id.file), text, depth);
    let (first, last) = (def.symbol.first, def.symbol.last);
    // The first row is the signature, which `sig:` already shows.
    let shown: Vec<&Row> = all
        .iter()
        .filter(|row| row.first >= first && row.last <= last)
        .skip(1)
        .collect();

    let edges = codebase.graph.uses(node);
    let mut calls: Vec<(usize, String)> = Vec::new();
    for used in def.uses.iter().filter(|used| used.role == Role::Call) {
        let mut found: Vec<&Edge> = edges
            .iter()
            .filter(|edge| edge.to != node && codebase.def(edge.to).symbol.name == used.name)
            .collect();
        found.sort_by(|a, b| b.weight.total_cmp(&a.weight));
        let Some(best) = found.first() else {
            continue;
        };
        let mark = if found.len() > 1 { "?" } else { "" };
        let note = format!("{}{mark}: {}", used.name, gist(codebase, best.to));
        for &line in &used.lines {
            calls.push((line, note.clone()));
        }
    }
    calls.sort();

    let mut rows: Vec<String> = Vec::new();
    for row in shown.iter().take(cap) {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let notes: Vec<&str> = calls
            .iter()
            .filter(|(line, _)| (row.first..=row.last).contains(line))
            .map(|(_, note)| note.as_str())
            .filter(|note| seen.insert(*note))
            .take(2)
            .collect();
        let mut line = format!("{}  {}", row.locator(), cut(row.text.trim_end(), 70));
        for (at, note) in notes.iter().enumerate() {
            line.push_str(if at == 0 { "  → " } else { " · " });
            line.push_str(note);
        }
        rows.push(fit(&line));
    }
    if shown.len() > cap {
        rows.push(format!("… {} more rows", shown.len() - cap));
    }
    rows
}

/// One source line for each place that calls the definition `node`, at most `cap` of them, the
/// callers that are not tests first in the order of `callers`.
pub(super) fn call_sites(
    workspace: Option<&dyn Workspace>,
    codebase: &Codebase,
    node: usize,
    callers: &[Edge],
    cap: usize,
) -> Vec<String> {
    let name = &codebase.def(node).symbol.name;
    let mut texts: HashMap<usize, Option<String>> = HashMap::new();
    let mut rows: Vec<String> = Vec::new();
    for edge in callers {
        let caller = codebase.def(edge.to);
        if in_test(caller) {
            continue;
        }
        let file = codebase.file_of(edge.to);
        let text = texts
            .entry(file)
            .or_insert_with(|| source_of(workspace, codebase.label(file)));
        let Some(text) = text else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for used in caller
            .uses
            .iter()
            .filter(|used| used.role == Role::Call && used.name == *name)
        {
            for &line in used.lines.iter().take(2) {
                if rows.len() >= cap {
                    return rows;
                }
                if let Some(source) = line.checked_sub(1).and_then(|at| lines.get(at)) {
                    let label = codebase.label(file);
                    rows.push(fit(&format!("{label}:{line}  {}", source.trim())));
                }
            }
        }
    }
    rows
}
