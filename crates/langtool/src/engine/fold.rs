//! Depth view: a file's structure with bodies below a nesting depth folded away.
//!
//! Source code folds by syntax block (any language with a [`Syntax`]), Markdown and TOML by
//! section (heading level), every other text file by indentation. All modes produce [`Row`]s, so
//! the tool handler pages and prints them the same way. The functions work on text; reading the
//! file belongs to the tool handler.
//!
//! Level `n` shows everything nested up to `n` levels deep:
//! - Code: a function, `if` or `match` body is one level. The bodies of `impl`, `trait`, `mod`
//!   and classes only hold declarations and add no level, so level 0 is an outline that lists
//!   methods. At level 0 doc comments and multi-line attributes are also cut to their first line.
//! - Markdown: the body of a `#` section is level 1, of a `##` section level 2, and so on.
//! - TOML: the body of a `[table]` is level 1, of `[a.b]` level 2, and so on.
//! - Other text: each deeper indentation is one level.

use std::fmt;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use tree_sitter::Node;

use crate::language::{Syntax, line_span, parse, registry};

/// Source lines `first..=last` (1-based) shown as one line of output: a single source line
/// as it is, or several lines folded into a placeholder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub first: usize,
    pub last: usize,
    pub text: String,
}

impl Row {
    /// `N` for a single line, `A..B` for a fold.
    pub fn locator(&self) -> String {
        if self.first == self.last {
            self.first.to_string()
        } else {
            format!("{}..{}", self.first, self.last)
        }
    }
}

/// How a file was folded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ast,
    Markdown,
    Toml,
    Indent,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Ast => "ast",
            Mode::Markdown => "markdown",
            Mode::Toml => "toml",
            Mode::Indent => "indent",
        }
    }
}

/// Nesting level for code/text folding (0..=16).
///
/// - Level `0`: An outline of declarations (impls, functions, classes, traits) with bodies folded away.
/// - Level `1..=16`: Nested blocks up to this depth are shown; deeper blocks are folded into `A..B` rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
pub struct FoldLevel(u32);

impl FoldLevel {
    pub const OUTLINE: Self = Self(0);
    pub const MAX_DEPTH: u32 = 16;

    /// Create a fold level, clamping to the maximum supported depth (16).
    pub const fn new(level: u32) -> Self {
        if level > Self::MAX_DEPTH {
            Self(Self::MAX_DEPTH)
        } else {
            Self(level)
        }
    }

    /// The outline level (0).
    pub const fn outline() -> Self {
        Self::OUTLINE
    }

    /// Whether this fold level represents the declaration outline (level 0).
    pub const fn is_outline(&self) -> bool {
        self.0 == 0
    }

    /// The depth value as a `u32`.
    pub const fn depth(&self) -> u32 {
        self.0
    }

    /// The depth value as a `usize` (clamped to 16).
    pub const fn as_usize(&self) -> usize {
        self.0 as usize
    }

    /// The depth value as a `u64`.
    pub const fn as_u64(&self) -> u64 {
        self.0 as u64
    }
}

impl Default for FoldLevel {
    fn default() -> Self {
        Self::OUTLINE
    }
}

impl fmt::Display for FoldLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for FoldLevel {
    fn from(val: u32) -> Self {
        Self::new(val)
    }
}

impl From<i32> for FoldLevel {
    fn from(val: i32) -> Self {
        Self::new(val.max(0) as u32)
    }
}

impl From<usize> for FoldLevel {
    fn from(val: usize) -> Self {
        Self::new(val as u32)
    }
}

impl From<u64> for FoldLevel {
    fn from(val: u64) -> Self {
        Self::new(val as u32)
    }
}

impl From<FoldLevel> for u32 {
    fn from(level: FoldLevel) -> Self {
        level.depth()
    }
}

impl From<FoldLevel> for u64 {
    fn from(level: FoldLevel) -> Self {
        level.as_u64()
    }
}

impl From<FoldLevel> for usize {
    fn from(level: FoldLevel) -> Self {
        level.as_usize()
    }
}

impl<'de> Deserialize<'de> for FoldLevel {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let n = crate::common::params::flex_u64(d)?;
        match n {
            Some(val) => Ok(Self::from(val)),
            None => Ok(Self::OUTLINE),
        }
    }
}

/// Fold the text of the file `path` to `max_depth`, choosing the mode by its extension. Code
/// that cannot be parsed and unknown file types fold by indentation, so every text file has a
/// view.
pub fn fold_file(path: &str, source: &str, max_depth: usize) -> (Mode, Vec<Row>) {
    if let Some(syntax) = registry::for_path(path)
        && let Some(rows) = fold(syntax, source, max_depth)
    {
        return (Mode::Ast, rows);
    }
    let lines: Vec<&str> = source.lines().collect();
    let extension = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("md" | "markdown" | "mdx") => (
            Mode::Markdown,
            fold_sections(&lines, &markdown_headings(&lines), max_depth),
        ),
        Some("toml") => (
            Mode::Toml,
            fold_sections(&lines, &toml_headings(&lines), max_depth),
        ),
        _ => (Mode::Indent, fold_indent(&lines, max_depth)),
    }
}

/// Fold every block nested deeper than `max_depth`. The outermost block counts as depth 1, so
/// `0` leaves only declarations. A fold keeps the text before the block opens, so a reader
/// still sees the signature. `None` when the source cannot be parsed.
pub fn fold(syntax: &dyn Syntax, source: &str, max_depth: usize) -> Option<Vec<Row>> {
    let tree = parse(syntax, source)?;
    let lines: Vec<&str> = source.lines().collect();
    let mut folds = Vec::new();
    let ctx = Ctx {
        syntax,
        source,
        lines: &lines,
        max_depth,
    };
    collect(tree.root_node(), 0, &ctx, &mut folds);
    Some(assemble(&lines, folds))
}

/// One output line per row: `N<tab>text`, or `A..B<tab>text` for a fold.
pub fn render(rows: &[Row]) -> String {
    rows.iter()
        .map(|row| format!("{}\t{}\n", row.locator(), row.text))
        .collect()
}

/// One row per source line, with each fold replacing the lines it covers. `folds` must be in
/// source order.
fn assemble(lines: &[&str], folds: Vec<Row>) -> Vec<Row> {
    let mut folds = folds.into_iter().peekable();
    let mut rows = Vec::new();
    let mut line = 1;
    while line <= lines.len() {
        while folds.next_if(|f| f.first < line).is_some() {}
        if let Some(fold) = folds.next_if(|f| f.first == line) {
            line = fold.last + 1;
            rows.push(fold);
        } else {
            rows.push(Row {
                first: line,
                last: line,
                text: lines[line - 1].to_string(),
            });
            line += 1;
        }
    }
    rows
}

struct Ctx<'a> {
    syntax: &'a dyn Syntax,
    source: &'a str,
    lines: &'a [&'a str],
    max_depth: usize,
}

fn push(out: &mut Vec<Row>, row: Row) {
    if out.last().is_none_or(|prev| row.first > prev.last) {
        out.push(row);
    }
}

fn collect(node: Node<'_>, depth: usize, ctx: &Ctx<'_>, out: &mut Vec<Row>) {
    let mut depth = depth;
    if ctx.syntax.block_kinds().contains(&node.kind()) && !ctx.syntax.is_container(node) {
        depth += 1;
        if depth > ctx.max_depth {
            if let Some(row) = fold_block(node, ctx) {
                push(out, row);
            }
            return;
        }
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
    let mut i = 0;
    while i < children.len() {
        let child = children[i];
        if ctx.max_depth == 0 {
            if ctx.syntax.is_doc(child, ctx.source) {
                let mut j = i;
                while j + 1 < children.len()
                    && ctx.syntax.is_doc(children[j + 1], ctx.source)
                    && line_span(children[j]).1 + 1 == line_span(children[j + 1]).0
                {
                    j += 1;
                }
                if j > i
                    && let Some(row) = fold_docs(child, children[j], ctx)
                {
                    push(out, row);
                }
                i = j + 1;
                continue;
            }
            if ctx.syntax.is_attribute(child) {
                if let Some(row) = fold_attribute(child, ctx) {
                    push(out, row);
                }
                i += 1;
                continue;
            }
        }
        collect(child, depth, ctx, out);
        i += 1;
    }
}

/// The fold for one block, or `None` when there is nothing to hide: a block without lines
/// of its own, or an inline body that shares its line with other code.
fn fold_block(node: Node<'_>, ctx: &Ctx<'_>) -> Option<Row> {
    let start = node.start_position();
    let end = node.end_position();
    let prefix = ctx.lines.get(start.row)?.get(..start.column)?;
    if ctx.source.as_bytes().get(node.start_byte()) == Some(&b'{') {
        // Braced body: keep the opening and closing text, hide the lines between.
        let (first, last) = (start.row + 1, end.row + 1);
        if last <= first + 1 {
            return None;
        }
        let suffix = ctx.lines.get(end.row)?.get(end.column..)?;
        let text = format!("{prefix}{}{suffix}", ctx.syntax.elision());
        Some(Row { first, last, text })
    } else {
        // Indented body: it owns whole lines, all of which are hidden. A single line stays,
        // because a row of one line could not be told from a source line.
        let (first, last) = line_span(node);
        if !prefix.trim().is_empty() || last <= first {
            return None;
        }
        let summary = ctx
            .syntax
            .body_summary(node, ctx.source)
            .map(|s| format!("{s} "))
            .unwrap_or_default();
        let text = format!("{prefix}{summary}{}", ctx.syntax.elision());
        Some(Row { first, last, text })
    }
}

/// A run of doc comment lines, cut to its first line.
fn fold_docs(first: Node<'_>, last: Node<'_>, ctx: &Ctx<'_>) -> Option<Row> {
    let (first_line, _) = line_span(first);
    let (_, last_line) = line_span(last);
    let text = ctx.lines.get(first_line - 1)?.to_string();
    Some(Row {
        first: first_line,
        last: last_line,
        text,
    })
}

/// An attribute written over several lines, cut to its first line.
fn fold_attribute(node: Node<'_>, ctx: &Ctx<'_>) -> Option<Row> {
    let (first, last) = line_span(node);
    if last <= first {
        return None;
    }
    let text = format!("{} ...", ctx.lines.get(first - 1)?);
    Some(Row { first, last, text })
}

/// Headings as `(zero-based line, level)`: ATX headings outside fenced code blocks.
fn markdown_headings(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if indent > 3 {
            continue;
        }
        if let Some(mark) = trimmed.chars().next().filter(|c| matches!(c, '`' | '~')) {
            let run = trimmed.chars().take_while(|&c| c == mark).count();
            if run >= 3 {
                match fence {
                    None => fence = Some((mark, run)),
                    Some((open, len)) if open == mark && run >= len => fence = None,
                    Some(_) => {}
                }
                continue;
            }
        }
        if fence.is_some() {
            continue;
        }
        let level = trimmed.chars().take_while(|&c| c == '#').count();
        let after = &trimmed[level..];
        if (1..=6).contains(&level) && (after.is_empty() || after.starts_with([' ', '\t'])) {
            out.push((i, level));
        }
    }
    out
}

/// Table headers as `(zero-based line, level)`; the level is the number of dotted name parts.
fn toml_headings(lines: &[&str]) -> Vec<(usize, usize)> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with('['))
        .filter_map(|(i, line)| {
            let inner = line.trim_start_matches('[');
            let name = inner.split(']').next()?;
            (!name.trim().is_empty()).then(|| (i, name.split('.').count()))
        })
        .collect()
}

/// Fold the body of every section whose heading level is deeper than `max_depth`. A section
/// runs to the next heading of the same or a higher level. Blank lines before the next heading
/// stay visible, and a body of fewer than two lines stays as it is.
fn fold_sections(lines: &[&str], headings: &[(usize, usize)], max_depth: usize) -> Vec<Row> {
    let mut folds = Vec::new();
    let mut covered = 0;
    for (i, &(at, level)) in headings.iter().enumerate() {
        if level <= max_depth || at < covered {
            continue;
        }
        let end = headings[i + 1..]
            .iter()
            .find(|&&(_, l)| l <= level)
            .map_or(lines.len(), |&(next, _)| next);
        let mut last = end;
        while last > at + 1 && lines[last - 1].trim().is_empty() {
            last -= 1;
        }
        if last >= at + 3 {
            folds.push(Row {
                first: at + 2,
                last,
                text: "...".to_string(),
            });
            covered = last;
        }
    }
    assemble(lines, folds)
}

/// Fold runs of lines indented deeper than `max_depth` levels. Each indentation deeper than the
/// line before it opens a level; blank lines belong to the run around them.
fn fold_indent(lines: &[&str], max_depth: usize) -> Vec<Row> {
    let mut stack: Vec<usize> = Vec::new();
    let mut folds = Vec::new();
    let mut run: Option<(usize, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let width = indent_width(line);
        while stack.last().is_some_and(|&top| top > width) {
            stack.pop();
        }
        if stack.last() != Some(&width) {
            stack.push(width);
        }
        if stack.len() - 1 > max_depth {
            run = Some(run.map_or((i, i), |(first, _)| (first, i)));
        } else if let Some(finished) = run.take() {
            push_indent_fold(&mut folds, lines, finished);
        }
    }
    if let Some(finished) = run {
        push_indent_fold(&mut folds, lines, finished);
    }
    assemble(lines, folds)
}

fn push_indent_fold(folds: &mut Vec<Row>, lines: &[&str], (first, last): (usize, usize)) {
    if last > first {
        let indent: String = lines[first]
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect();
        folds.push(Row {
            first: first + 1,
            last: last + 1,
            text: format!("{indent}..."),
        });
    }
}

fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}
