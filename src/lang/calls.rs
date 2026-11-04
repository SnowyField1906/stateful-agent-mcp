//! Call graph: which functions a function calls and which functions call it.
//!
//! Calls are matched to definitions by name only, across every indexed file. Only the
//! [`Syntax`] trait is used; walking directories, reading files and paginating results belong
//! to the tool handler.

use tree_sitter::Node;

use super::aliases::{aliases as file_aliases, resolve};
use super::syntax::{Alias, Syntax, line_span, parse};

/// A function or method definition and the names it calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnInfo {
    pub name: String,
    /// The enclosing definition as `kind name`, empty at the top level.
    pub scope: String,
    /// 1-based first and last line.
    pub first: usize,
    pub last: usize,
    /// Distinct names called in the body, in source order. A call inside a nested function
    /// belongs to the nested function only.
    /// Names are the originals behind import aliases (`use a::b as c; c()` calls `b`).
    pub calls: Vec<String>,
}

/// A function together with the file it is defined in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub file: String,
    pub func: FnInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Incoming,
    Outgoing,
    Both,
}

pub struct Query<'a> {
    pub symbol: &'a str,
    /// Only definitions in this file.
    pub file: Option<&'a str>,
    /// Only the definition that contains this 1-based line.
    pub line: Option<usize>,
    pub direction: Direction,
    /// Levels to follow, clamped to `1..=MAX_DEPTH`.
    pub depth: usize,
}

pub const MAX_DEPTH: usize = 3;

/// All functions in `source`, in source order. `None` when it cannot be parsed.
pub fn functions(syntax: &dyn Syntax, source: &str) -> Option<Vec<FnInfo>> {
    let tree = parse(syntax, source)?;
    let imports = file_aliases(syntax, source)?;
    let mut out = Vec::new();
    walk(tree.root_node(), "", None, syntax, source, &imports, &mut out);
    Some(out)
}

/// Render the call tree of every definition of `query.symbol`. `None` when there is none.
pub fn render(index: &[Entry], query: &Query<'_>) -> Option<String> {
    let depth = query.depth.clamp(1, MAX_DEPTH);
    let targets: Vec<&Entry> = index
        .iter()
        .filter(|e| {
            e.func.name == query.symbol
                && query.file.is_none_or(|f| e.file == f)
                && query.line.is_none_or(|l| (e.func.first..=e.func.last).contains(&l))
        })
        .collect();
    if targets.is_empty() {
        return None;
    }
    let mut out = String::new();
    for target in targets {
        out.push_str(&label(target));
        out.push('\n');
        let path = &mut vec![target.func.name.clone()];
        if query.direction != Direction::Incoming {
            out.push_str("outgoing:\n");
            outgoing(index, target, depth, path, &mut out);
        }
        if query.direction != Direction::Outgoing {
            out.push_str("incoming:\n");
            incoming(index, target, depth, path, &mut out);
        }
    }
    Some(out)
}

/// `name (in scope)<tab>file:first..last`.
fn label(entry: &Entry) -> String {
    let f = &entry.func;
    let scope = if f.scope.is_empty() { String::new() } else { format!(" (in {})", f.scope) };
    format!("{}{}\t{}:{}..{}", f.name, scope, entry.file, f.first, f.last)
}

fn outgoing(index: &[Entry], from: &Entry, remaining: usize, path: &mut Vec<String>, out: &mut String) {
    let indent = "  ".repeat(path.len());
    for call in &from.func.calls {
        let defs: Vec<&Entry> = index.iter().filter(|e| &e.func.name == call).collect();
        if defs.is_empty() {
            out.push_str(&format!("{indent}{call}\t(external)\n"));
            continue;
        }
        for def in defs {
            descend(index, def, remaining, path, out, &indent, outgoing);
        }
    }
}

fn incoming(index: &[Entry], to: &Entry, remaining: usize, path: &mut Vec<String>, out: &mut String) {
    let indent = "  ".repeat(path.len());
    for caller in index.iter().filter(|e| e.func.calls.contains(&to.func.name)) {
        descend(index, caller, remaining, path, out, &indent, incoming);
    }
}

type Step = fn(&[Entry], &Entry, usize, &mut Vec<String>, &mut String);

/// Print `entry`, then follow it one level deeper unless it closes a cycle or the depth is used.
fn descend(
    index: &[Entry],
    entry: &Entry,
    remaining: usize,
    path: &mut Vec<String>,
    out: &mut String,
    indent: &str,
    step: Step,
) {
    if path.contains(&entry.func.name) {
        out.push_str(&format!("{indent}{}\t(cycle)\n", label(entry)));
        return;
    }
    out.push_str(&format!("{indent}{}\n", label(entry)));
    if remaining > 1 {
        path.push(entry.func.name.clone());
        step(index, entry, remaining - 1, path, out);
        path.pop();
    }
}

fn walk(
    node: Node<'_>,
    scope: &str,
    current: Option<usize>,
    syntax: &dyn Syntax,
    source: &str,
    imports: &[Alias],
    out: &mut Vec<FnInfo>,
) {
    let mut inner = scope.to_string();
    let mut cur = current;
    if let Some(def) = syntax.definition(node, source) {
        inner = format!("{} {}", def.kind, def.name);
        if syntax.is_function(def.kind) {
            let (first, last) = line_span(node);
            out.push(FnInfo { name: def.name, scope: scope.to_string(), first, last, calls: Vec::new() });
            cur = Some(out.len() - 1);
        }
    } else if let (Some(i), Some(name)) = (current, syntax.call_name(node, source)) {
        let name = resolve(imports, &name).to_string();
        let calls = &mut out[i].calls;
        if !calls.contains(&name) {
            calls.push(name);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, &inner, cur, syntax, source, imports, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::syntax::python::PythonSyntax;
    use crate::lang::syntax::rust::RustSyntax;

    const RUST: &str = "fn a() { b(); ext(); }\nfn b() { c(); }\nfn c() { a(); }\n";

    fn index(file: &str, syntax: &dyn Syntax, source: &str) -> Vec<Entry> {
        functions(syntax, source)
            .unwrap()
            .into_iter()
            .map(|func| Entry { file: file.to_string(), func })
            .collect()
    }

    fn query(symbol: &str, direction: Direction, depth: usize) -> Query<'_> {
        Query { symbol, file: None, line: None, direction, depth }
    }

    #[test]
    fn collects_calls_per_function() {
        let fns = functions(&RustSyntax, RUST).unwrap();
        let calls: Vec<(&str, &[String])> = fns.iter().map(|f| (f.name.as_str(), f.calls.as_slice())).collect();
        assert_eq!(calls[0], ("a", ["b".to_string(), "ext".to_string()].as_slice()));
        assert_eq!(calls[1], ("b", ["c".to_string()].as_slice()));
    }

    #[test]
    fn outgoing_follows_depth_and_marks_external() {
        let idx = index("x.rs", &RustSyntax, RUST);
        let one = render(&idx, &query("a", Direction::Outgoing, 1)).unwrap();
        assert_eq!(one, "a\tx.rs:1..1\noutgoing:\n  b\tx.rs:2..2\n  ext\t(external)\n");
        let two = render(&idx, &query("a", Direction::Outgoing, 2)).unwrap();
        assert_eq!(
            two,
            "a\tx.rs:1..1\noutgoing:\n  b\tx.rs:2..2\n    c\tx.rs:3..3\n  ext\t(external)\n"
        );
    }

    #[test]
    fn incoming_detects_cycles() {
        let idx = index("x.rs", &RustSyntax, RUST);
        let out = render(&idx, &query("a", Direction::Incoming, 3)).unwrap();
        assert_eq!(
            out,
            "a\tx.rs:1..1\nincoming:\n  c\tx.rs:3..3\n    b\tx.rs:2..2\n      a\tx.rs:1..1\t(cycle)\n"
        );
    }

    #[test]
    fn both_directions_and_unknown_symbol() {
        let idx = index("x.rs", &RustSyntax, RUST);
        let out = render(&idx, &query("b", Direction::Both, 1)).unwrap();
        assert_eq!(out, "b\tx.rs:2..2\noutgoing:\n  c\tx.rs:3..3\nincoming:\n  a\tx.rs:1..1\n");
        assert!(render(&idx, &query("nope", Direction::Both, 1)).is_none());
    }

    #[test]
    fn calls_through_import_aliases_count_as_calls_of_the_original() {
        let user = "use m::target as t;\nfn run() { t(); }\n";
        let mut idx = index("u.rs", &RustSyntax, user);
        idx.extend(index("d.rs", &RustSyntax, "fn target() {}\n"));
        let out = render(&idx, &query("target", Direction::Incoming, 1)).unwrap();
        assert_eq!(out, "target\td.rs:1..1\nincoming:\n  run\tu.rs:2..2\n");
        let out = render(&idx, &query("run", Direction::Outgoing, 1)).unwrap();
        assert_eq!(out, "run\tu.rs:2..2\noutgoing:\n  target\td.rs:1..1\n");
    }

    #[test]
    fn narrows_by_file_and_line_and_reads_python_methods() {
        let py = "class A:\n    def f(self):\n        g()\n\ndef g():\n    pass\n";
        let mut idx = index("a.py", &PythonSyntax, py);
        idx.extend(index("b.py", &PythonSyntax, "def g():\n    pass\n"));
        let q = Query { symbol: "g", file: Some("b.py"), line: None, direction: Direction::Incoming, depth: 1 };
        assert_eq!(render(&idx, &q).unwrap(), "g\tb.py:1..2\nincoming:\n  f (in class A)\ta.py:2..3\n");
        let q = Query { symbol: "g", file: None, line: Some(5), direction: Direction::Outgoing, depth: 1 };
        assert_eq!(render(&idx, &q).unwrap(), "g\ta.py:5..6\noutgoing:\n");
    }
}
