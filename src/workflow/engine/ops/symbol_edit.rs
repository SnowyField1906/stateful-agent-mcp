//! Whole-definition edits: `ReplaceSymbol`, `InsertBeforeSymbol` and `InsertAfterSymbol`.
//!
//! An edit works on whole lines of one definition. Its unit is the definition together with the
//! doc comments and attributes written directly above it. The edit returns the new text and the
//! [`PatchHunk`] that describes it; the turn ledger is built from before/after texts, so it only
//! ever records ordinary hunks.

use crate::common::LineNumber;
use crate::langtool::engine::symbols::{self, Symbol};
use crate::langtool::engine::syntax::{Syntax, parse, registry};

use super::PatchHunk;
use super::patch::byte_offset_of_line;

/// Where `content` goes relative to the definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Replace the definition. Doc comments and attributes above it stay unless `content`
    /// starts with its own.
    Replace,
    /// Insert above the definition's doc comments and attributes, then a blank line.
    Before,
    /// Insert below the definition, after a blank line.
    After,
}

/// The new text of a file and the hunk that produces it.
#[derive(Debug)]
pub struct SymbolEdit {
    pub text: String,
    pub hunk: PatchHunk,
    /// The edit left a Tree-sitter parse error in a file that had none before it.
    pub adds_syntax_error: bool,
}

/// The one definition `symbol` names in `text`, or why there is not exactly one.
pub(super) fn definition(path: &str, text: &str, symbol: &str) -> Result<Symbol, String> {
    let syntax = registry::for_path(path).ok_or_else(|| {
        "`symbol` needs a file in a language with a registered Tree-sitter grammar".to_string()
    })?;
    let all = symbols::symbols(syntax, text)
        .ok_or_else(|| "`symbol` needs a file that can be parsed".to_string())?;
    symbols::find_one(&all, symbol).cloned()
}

/// Edit the definition `symbol` of `text` (the file at `path`) with `content`. `content` is one
/// or more whole definitions; it is moved as a block to the indentation of the definition and
/// keeps its own relative indentation.
pub fn edit_symbol(
    path: &str,
    text: &str,
    symbol: &str,
    content: &str,
    placement: Placement,
) -> Result<SymbolEdit, String> {
    let hit = definition(path, text, symbol)?;
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let body = reindent(content, leading_ws(line_at(text, hit.head)), eol)
        .ok_or_else(|| "`content` is empty".to_string())?;

    let (from, to) = match placement {
        Placement::Replace if starts_with_decoration(content) => (hit.first, hit.last + 1),
        Placement::Replace => (hit.head, hit.last + 1),
        Placement::Before => (hit.first, hit.first),
        Placement::After => (hit.last + 1, hit.last + 1),
    };
    let start = byte_offset_of_line(
        text,
        LineNumber::new(from as u64).unwrap_or(LineNumber::ONE),
    );
    let end = byte_offset_of_line(text, LineNumber::new(to as u64).unwrap_or(LineNumber::ONE));
    // The edit reaches the end of a file without a final newline: keep it that way.
    let open_end = end == text.len() && !text.is_empty() && !text.ends_with('\n');
    let tail = if open_end { "" } else { eol };
    let region = match placement {
        Placement::Replace => format!("{body}{tail}"),
        Placement::Before => format!("{body}{eol}{eol}"),
        Placement::After if open_end => format!("{eol}{eol}{body}"),
        Placement::After => format!("{eol}{body}{eol}"),
    };

    let hunk = PatchHunk {
        start_line: LineNumber::new(from as u64),
        old_text: text[start..end].to_string(),
        new_text: region.clone(),
        symbol: None,
    };
    let mut out = String::with_capacity(text.len() + region.len());
    out.push_str(&text[..start]);
    out.push_str(&region);
    out.push_str(&text[end..]);
    let adds_syntax_error = registry::for_path(path)
        .is_some_and(|syntax| has_syntax_errors(syntax, &out) && !has_syntax_errors(syntax, text));
    Ok(SymbolEdit {
        text: out,
        hunk,
        adds_syntax_error,
    })
}

/// True when `text` cannot be parsed or its Tree-sitter tree has an error node.
pub fn has_syntax_errors(syntax: &dyn Syntax, text: &str) -> bool {
    parse(syntax, text).is_none_or(|tree| tree.root_node().has_error())
}

/// True when the first line of `content` is a doc comment, attribute or decorator, so the caller
/// supplies the definition's whole unit.
fn starts_with_decoration(content: &str) -> bool {
    content
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim_start)
        .is_some_and(|line| {
            ["///", "/**", "#[", "@"]
                .iter()
                .any(|p| line.starts_with(p))
        })
}

/// The text of 1-based `line`, without its terminator; empty past the end.
fn line_at(text: &str, line: usize) -> &str {
    text[byte_offset_of_line(
        text,
        LineNumber::new(line as u64).unwrap_or(LineNumber::ONE),
    )..]
        .lines()
        .next()
        .unwrap_or("")
}

/// The spaces and tabs a line starts with.
fn leading_ws(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// `content` without leading and trailing blank lines, moved as a block so its least indented
/// line starts with `indent`, with lines joined by `eol`. `None` when nothing is left.
fn reindent(content: &str, indent: &str, eol: &str) -> Option<String> {
    let mut lines: Vec<&str> = content.lines().collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let skip = lines
        .iter()
        .take_while(|line| line.trim().is_empty())
        .count();
    let lines = &lines[skip..];
    let least = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| leading_ws(line).len())
        .min()?;
    let moved: Vec<String> = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                format!("{indent}{}", &line[least..])
            }
        })
        .collect();
    Some(moved.join(eol))
}
