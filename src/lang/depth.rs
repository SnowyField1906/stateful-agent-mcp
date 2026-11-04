//! Depth view: a file's structure with block bodies below a nesting depth folded away.
//!
//! Only the [`Syntax`] trait is used, so every language with a `Syntax` gets the view. The
//! functions work on text; reading the file and paginating lines belong to the tool handler.

use tree_sitter::Node;

use super::syntax::{Syntax, line_span, parse};

/// Source lines `first..=last` (1-based) shown as one line of output: a single source line
/// as it is, or several lines folded into a placeholder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub first: usize,
    pub last: usize,
    pub text: String,
}

/// Fold every block nested deeper than `max_depth`. The outermost block counts as depth 1, so
/// `0` leaves only top-level declarations. A fold keeps the text before the block opens, so a
/// reader still sees the signature. `None` when the source cannot be parsed.
pub fn fold(syntax: &dyn Syntax, source: &str, max_depth: usize) -> Option<Vec<Row>> {
    let tree = parse(syntax, source)?;
    let lines: Vec<&str> = source.lines().collect();
    let mut folds = Vec::new();
    let ctx = Ctx { syntax, source, lines: &lines, max_depth };
    collect(tree.root_node(), 0, &ctx, &mut folds);

    let mut folds = folds.into_iter().peekable();
    let mut rows = Vec::new();
    let mut line = 1;
    while line <= lines.len() {
        while folds.next_if(|f| f.first < line).is_some() {}
        if let Some(fold) = folds.next_if(|f| f.first == line) {
            line = fold.last + 1;
            rows.push(fold);
        } else {
            rows.push(Row { first: line, last: line, text: lines[line - 1].to_string() });
            line += 1;
        }
    }
    Some(rows)
}

/// One output line per row: `N<tab>text`, or `A..B<tab>text` for a fold.
pub fn render(rows: &[Row]) -> String {
    let mut out = String::new();
    for row in rows {
        if row.first == row.last {
            out.push_str(&format!("{}\t{}\n", row.first, row.text));
        } else {
            out.push_str(&format!("{}..{}\t{}\n", row.first, row.last, row.text));
        }
    }
    out
}

struct Ctx<'a> {
    syntax: &'a dyn Syntax,
    source: &'a str,
    lines: &'a [&'a str],
    max_depth: usize,
}

fn collect(node: Node<'_>, depth: usize, ctx: &Ctx<'_>, out: &mut Vec<Row>) {
    let mut depth = depth;
    if ctx.syntax.block_kinds().contains(&node.kind()) {
        depth += 1;
        if depth > ctx.max_depth {
            if let Some(row) = fold_block(node, ctx)
                && out.last().is_none_or(|prev| row.first > prev.last)
            {
                out.push(row);
            }
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, depth, ctx, out);
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
        let text = format!("{prefix}{}{suffix}", ctx.syntax.elided(first + 1, last - 1));
        Some(Row { first, last, text })
    } else {
        // Indented body: it owns whole lines, all of which are hidden.
        let (first, last) = line_span(node);
        if !prefix.trim().is_empty() || last < first {
            return None;
        }
        Some(Row { first, last, text: format!("{prefix}{}", ctx.syntax.elided(first, last)) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::syntax::python::PythonSyntax;
    use crate::lang::syntax::rust::RustSyntax;

    const RUST: &str = "struct P {\n    x: i32,\n    y: i32,\n}\n\nimpl P {\n    fn new() -> Self {\n        let a = 1;\n        P { x: a, y: 2 }\n    }\n}\n";

    const PYTHON: &str = "class A:\n    def f(self):\n        x = 1\n        return x\n\ndef g():\n    pass\n";

    fn view(syntax: &dyn Syntax, source: &str, depth: usize) -> String {
        render(&fold(syntax, source, depth).expect("parse"))
    }

    #[test]
    fn rust_folds_bodies_below_depth() {
        let out = view(&RustSyntax, RUST, 1);
        assert!(out.contains("1\tstruct P {\n"), "{out}");
        assert!(out.contains("2\t    x: i32,\n"), "{out}");
        assert!(out.contains("7..10\t    fn new() -> Self { ... /* 8..9 */ }\n"), "{out}");
        assert!(out.contains("11\t}\n"), "{out}");
        assert!(!out.contains("let a"), "{out}");
    }

    #[test]
    fn rust_depth_zero_keeps_only_declarations() {
        let out = view(&RustSyntax, RUST, 0);
        assert!(out.contains("1..4\tstruct P { ... /* 2..3 */ }\n"), "{out}");
        assert!(out.contains("6..11\timpl P { ... /* 7..10 */ }\n"), "{out}");
        assert!(!out.contains("fn new"), "{out}");
    }

    #[test]
    fn python_folds_indented_bodies() {
        let out = view(&PythonSyntax, PYTHON, 1);
        assert!(out.contains("2\t    def f(self):\n"), "{out}");
        assert!(out.contains("3..4\t        ... # 3..4\n"), "{out}");
        assert!(out.contains("7\t    pass\n"), "{out}");
        assert!(!out.contains("return x"), "{out}");
    }

    #[test]
    fn deep_enough_view_is_unchanged() {
        let rows = fold(&RustSyntax, RUST, 8).expect("parse");
        assert_eq!(rows.len(), RUST.lines().count());
        assert!(rows.iter().all(|r| r.first == r.last));
    }
}
