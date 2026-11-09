//! Where a failed `Patch` was aimed. The server reads it to hint, after a repeated miss on one
//! file, at the lines to read again.

use std::cmp::Reverse;

use crate::common::AgentError;
use crate::engine::Project;
use crate::engine::ops::overlay::read_text_opt;
use crate::engine::ops::{FileOp, PatchHunk};
use crate::langtool::engine::symbols::{self, Symbol};
use crate::language::registry;

/// Lines on each side of an aimed line that are suggested when no definition holds it.
const MARGIN: usize = 10;
/// Fewest non-blank characters of a quoted line that can stand for where its hunk was aimed.
const MIN_ANCHOR: usize = 12;

/// The file of a failed `Patch` and where its failing hunk was aimed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchMiss {
    pub path: String,
    /// 1-based first and last line to read again; `None` when the hunk cannot be placed at all.
    pub lines: Option<(usize, usize)>,
    /// The definition around those lines, written as `ReplaceSymbol` takes it (`Type::name`).
    pub symbol: Option<String>,
}

impl Project {
    /// The miss that `err` reports: `Some` for a patch conflict. A failed batch writes nothing, so
    /// the file on disk is the text the hunk was aimed at.
    pub(crate) fn patch_miss(&self, err: &AgentError, ops: &[FileOp]) -> Option<PatchMiss> {
        let AgentError::PatchConflict { path, .. } = err else {
            return None;
        };
        let text = read_text_opt(&self.root, path)
            .ok()
            .flatten()
            .unwrap_or_default();
        let hunks: Vec<&PatchHunk> = ops
            .iter()
            .filter_map(|op| match op {
                FileOp::Patch { path: named, hunks } if named.trim_start_matches("./") == path => {
                    Some(hunks.iter())
                }
                _ => None,
            })
            .flatten()
            .collect();
        let failing = hunks
            .iter()
            .find(|hunk| !text.contains(hunk.old_text.as_str()))
            .or(hunks.first())
            .copied();
        let (lines, symbol) = failing.map_or((None, None), |hunk| aim(path, &text, hunk));
        Some(PatchMiss {
            path: path.clone(),
            lines,
            symbol,
        })
    }
}

/// The lines to read again for `hunk` and the definition that holds them: the innermost
/// definition around the aimed line, else the lines around it.
fn aim(path: &str, text: &str, hunk: &PatchHunk) -> (Option<(usize, usize)>, Option<String>) {
    let all = registry::for_path(path)
        .and_then(|syntax| symbols::symbols(syntax, text))
        .unwrap_or_default();
    let Some(line) = aimed_line(text, &all, hunk) else {
        return (None, None);
    };
    if let Some(symbol) = enclosing(&all, line) {
        return (Some((symbol.first, symbol.last)), Some(qualified(symbol)));
    }
    let total = text.lines().count();
    if line > total {
        return (None, None);
    }
    let window = (
        line.saturating_sub(MARGIN).max(1),
        (line + MARGIN).min(total),
    );
    (Some(window), None)
}

/// The 1-based line a hunk was aimed at: its `symbol`, its `start_line`, or else the one line of
/// the file that its longest quoted line names.
fn aimed_line(text: &str, all: &[Symbol], hunk: &PatchHunk) -> Option<usize> {
    if let Some(name) = hunk
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        && let Ok(found) = symbols::find_one(all, name)
    {
        return Some(found.head);
    }
    if let Some(line) = hunk.start_line {
        return Some(line.get() as usize);
    }
    anchor_line(text, &hunk.old_text)
}

/// The line of `text` that the longest quoted line of `old` names, when the quote is one line of
/// the file and no other.
fn anchor_line(text: &str, old: &str) -> Option<usize> {
    let mut quoted: Vec<&str> = old
        .lines()
        .map(str::trim)
        .filter(|line| line.chars().filter(|c| !c.is_whitespace()).count() >= MIN_ANCHOR)
        .collect();
    quoted.sort_by_key(|line| Reverse(line.len()));
    quoted.into_iter().find_map(|quote| {
        let mut hits = text
            .lines()
            .enumerate()
            .filter(|(_, line)| line.trim() == quote);
        match (hits.next(), hits.next()) {
            (Some((index, _)), None) => Some(index + 1),
            _ => None,
        }
    })
}

/// The innermost definition whose lines hold `line`.
fn enclosing(all: &[Symbol], line: usize) -> Option<&Symbol> {
    all.iter()
        .filter(|symbol| symbol.first <= line && line <= symbol.last)
        .max_by_key(|symbol| (symbol.first, Reverse(symbol.last)))
}

/// The name that `ReplaceSymbol` accepts for `symbol`: `Type::name` inside a type or module.
fn qualified(symbol: &Symbol) -> String {
    if symbol.scope.is_empty() {
        symbol.name.clone()
    } else {
        format!("{}::{}", symbol.scope, symbol.name)
    }
}
