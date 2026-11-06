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

use std::path::Path;

use tree_sitter::Node;

use super::syntax::{Syntax, line_span, parse, registry};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langtool::engine::syntax::python::PythonSyntax;
    use crate::langtool::engine::syntax::rust::RustSyntax;

    const RUST: &str = "struct P {\n    x: i32,\n    y: i32,\n}\n\nimpl P {\n    fn new() -> Self {\n        let a = 1;\n        P { x: a, y: 2 }\n    }\n}\n";

    const NESTED: &str =
        "fn f() {\n    let a = 1;\n    if a > 0 {\n        g();\n        h();\n    }\n}\n";

    const DOCS: &str = "/// First line.\n/// Second line.\n/// Third.\n#[derive(\n    Debug,\n)]\nstruct S;\n\nfn f() {}\n";

    const PYTHON: &str =
        "class A:\n    def f(self):\n        x = 1\n        return x\n\ndef g():\n    pass\n";

    const MARKDOWN: &str = "# Title\nintro\n\n## A\na1\na2\n\n### A.1\ndeep\ndeeper\n\n## B\nb1\nb2\n\n```\n# not a heading\n```\n";

    const TOML: &str = "[package]\nname = \"x\"\nversion = \"1\"\n\n[dependencies]\na = 1\nb = 2\n\n[dependencies.c]\nfeatures = [\"x\"]\nopt = true\n";

    const JSON: &str = "{\n  \"a\": {\n    \"b\": 1,\n    \"c\": 2\n  },\n  \"d\": 3\n}\n";

    fn view(syntax: &dyn Syntax, source: &str, depth: usize) -> String {
        render(&fold(syntax, source, depth).expect("parse"))
    }

    fn file_view(path: &str, source: &str, depth: usize) -> (Mode, String) {
        let (mode, rows) = fold_file(path, source, depth);
        (mode, render(&rows))
    }

    #[test]
    fn level_zero_is_an_outline_that_lists_methods() {
        let out = view(&RustSyntax, RUST, 0);
        assert!(out.contains("1..4\tstruct P { ... }\n"), "{out}");
        assert!(out.contains("6\timpl P {\n"), "{out}");
        assert!(
            out.contains("7..10\t    fn new() -> Self { ... }\n"),
            "{out}"
        );
        assert!(out.contains("11\t}\n"), "{out}");
        assert!(!out.contains("let a"), "{out}");
    }

    #[test]
    fn each_level_shows_one_more_block_depth() {
        let zero = view(&RustSyntax, NESTED, 0);
        assert_eq!(zero, "1..7\tfn f() { ... }\n");
        let one = view(&RustSyntax, NESTED, 1);
        assert!(one.contains("2\t    let a = 1;\n"), "{one}");
        assert!(one.contains("3..6\t    if a > 0 { ... }\n"), "{one}");
        assert!(!one.contains("g();"), "{one}");
        let two = view(&RustSyntax, NESTED, 2);
        assert!(two.contains("4\t        g();\n"), "{two}");
    }

    #[test]
    fn level_zero_cuts_docs_and_attributes_to_their_first_line() {
        let zero = view(&RustSyntax, DOCS, 0);
        assert!(zero.contains("1..3\t/// First line.\n"), "{zero}");
        assert!(zero.contains("4..6\t#[derive( ...\n"), "{zero}");
        assert!(
            !zero.contains("Second") && !zero.contains("Debug"),
            "{zero}"
        );
        let one = view(&RustSyntax, DOCS, 1);
        assert!(one.contains("2\t/// Second line.\n"), "{one}");
        assert!(one.contains("5\t    Debug,\n"), "{one}");
    }

    #[test]
    fn python_folds_indented_bodies_and_class_bodies_add_no_level() {
        let out = view(&PythonSyntax, PYTHON, 0);
        assert!(out.contains("1\tclass A:\n"), "{out}");
        assert!(out.contains("2\t    def f(self):\n"), "{out}");
        assert!(out.contains("3..4\t        ...\n"), "{out}");
        assert!(out.contains("7\t    pass\n"), "{out}");
        assert!(!out.contains("return x"), "{out}");
    }

    #[test]
    fn python_folds_show_the_docstring() {
        let src = "def f():\n    \"\"\"Do the thing.\n\n    More.\n    \"\"\"\n    return 1\n";
        let out = view(&PythonSyntax, src, 0);
        assert!(
            out.contains("2..6\t    \"\"\"Do the thing.\"\"\" ...\n"),
            "{out}"
        );
    }

    #[test]
    fn deep_enough_view_is_unchanged() {
        let rows = fold(&RustSyntax, RUST, 8).expect("parse");
        assert_eq!(rows.len(), RUST.lines().count());
        assert!(rows.iter().all(|r| r.first == r.last));
    }

    #[test]
    fn markdown_folds_sections_and_ignores_headings_in_code_fences() {
        let (mode, one) = file_view("README.md", MARKDOWN, 1);
        assert_eq!(mode, Mode::Markdown);
        assert!(
            one.contains("4\t## A\n") && one.contains("5..10\t...\n"),
            "{one}"
        );
        assert!(
            one.contains("12\t## B\n") && one.contains("13..18\t...\n"),
            "{one}"
        );
        assert!(!one.contains("deep"), "{one}");
        let (_, zero) = file_view("README.md", MARKDOWN, 0);
        assert_eq!(zero, "1\t# Title\n2..18\t...\n");
        let (_, two) = file_view("README.md", MARKDOWN, 2);
        assert!(
            two.contains("8\t### A.1\n") && two.contains("9..10\t...\n"),
            "{two}"
        );
    }

    #[test]
    fn toml_folds_tables_by_dotted_depth() {
        let (mode, zero) = file_view("Cargo.toml", TOML, 0);
        assert_eq!(mode, Mode::Toml);
        assert!(
            zero.contains("1\t[package]\n") && zero.contains("2..3\t...\n"),
            "{zero}"
        );
        assert!(
            zero.contains("5\t[dependencies]\n") && zero.contains("6..11\t...\n"),
            "{zero}"
        );
        let (_, one) = file_view("Cargo.toml", TOML, 1);
        assert!(
            one.contains("9\t[dependencies.c]\n") && one.contains("10..11\t...\n"),
            "{one}"
        );
    }

    #[test]
    fn other_text_folds_by_indentation() {
        let (mode, one) = file_view("data.json", JSON, 1);
        assert_eq!(mode, Mode::Indent);
        assert!(
            one.contains("3..4\t    ...\n") && one.contains("5\t  },\n"),
            "{one}"
        );
        let (_, zero) = file_view("data.json", JSON, 0);
        assert_eq!(zero, "1\t{\n2..6\t  ...\n7\t}\n");
        let (_, deep) = file_view("data.json", JSON, 5);
        assert_eq!(
            deep,
            JSON.lines()
                .enumerate()
                .map(|(i, l)| format!("{}\t{l}\n", i + 1))
                .collect::<String>()
        );
    }
}
