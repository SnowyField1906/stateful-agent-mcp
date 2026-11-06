//! Call graph: which functions a function calls and which functions call it.
//!
//! A call is matched to definitions by name and by how it was written (see [`CallKind`]):
//! `Self::new()` and `self.run()` stay in the caller's type, `Type::new()` goes to that type,
//! `module::run()` goes to free functions of that module, and `run()` goes to free functions.
//! Rust `value.run()` calls use an inferred receiver type when available; unknown receivers
//! remain possible method targets marked with `?`. Every edge ends with `@line`, the lines of
//! the calling function that make the call. Only the [`Syntax`] trait is used; walking
//! directories, reading files and paginating results belong to the tool handler.

use std::path::Path;

use tree_sitter::Node;

use super::aliases::{aliases as file_aliases, resolve};
use super::name_match::ranked_names;
use super::syntax::{Alias, CallKind, CallSite, Lineage, SymbolSelector, Syntax, parse, unit_span};

/// A function or method definition and the calls it makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnInfo {
    pub name: String,
    /// The enclosing definitions, outermost first; shown as a qualified path (`east::Token`).
    /// Empty at the top level.
    pub scope: Lineage,
    /// 1-based first and last line, including the doc comments, attributes and decorators
    /// written directly above the definition.
    pub first: usize,
    pub last: usize,
    /// Distinct calls in the body, in source order, each with the lines it is made on. A call
    /// inside a nested function belongs to the nested function only. Names are the originals
    /// behind import aliases (`use a::b as c; c()` calls `b`).
    pub calls: Vec<Call>,
}

/// A distinct call in a function body and where it is made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub site: CallSite,
    /// 1-based lines the call is made on, in source order: where each call expression starts.
    pub lines: Vec<usize>,
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
    /// Print every call that has no definition in the index. Otherwise they are summarized in
    /// one line per function.
    pub show_external: bool,
    /// Include callers and callees that are tests. A queried function that is itself a test
    /// always shows what it uses.
    pub include_tests: bool,
}

pub const MAX_DEPTH: usize = 3;

/// External names named in a summary line before the rest is counted.
const EXTERNAL_SHOWN: usize = 6;

/// Lines named after `@` for one call before the rest are counted.
const LINES_SHOWN: usize = 3;

/// All functions in `source`, in source order. `None` when it cannot be parsed.
pub fn functions(syntax: &dyn Syntax, source: &str) -> Option<Vec<FnInfo>> {
    let tree = parse(syntax, source)?;
    let imports = file_aliases(syntax, source)?;
    let mut out = Vec::new();
    walk(
        tree.root_node(),
        &mut Lineage::default(),
        None,
        syntax,
        source,
        &imports,
        &mut out,
    );
    Some(out)
}

/// Function names that resemble `symbol`, for the message when it has no definition.
pub fn suggest(index: &[Entry], symbol: &str) -> Vec<String> {
    ranked_names(
        index.iter().map(|entry| entry.func.name.clone()),
        SymbolSelector::parse(symbol).name,
        5,
        true,
    )
    .into_iter()
    .map(|(_, name)| name)
    .collect()
}

/// Render the call tree of every definition of `query.symbol`, which may be qualified
/// (`Type::method`, see [`SymbolSelector`]). `None` when there is none.
pub fn render(index: &[Entry], query: &Query<'_>) -> Option<String> {
    let depth = query.depth.clamp(1, MAX_DEPTH);
    let selector = SymbolSelector::parse(query.symbol);
    let targets: Vec<&Entry> = index
        .iter()
        .filter(|e| {
            e.func.name == selector.name
                && selector.matches_scope(&e.func.scope)
                && query.file.is_none_or(|f| e.file == f)
                && query
                    .line
                    .is_none_or(|l| (e.func.first..=e.func.last).contains(&l))
        })
        .collect();
    if targets.is_empty() {
        return None;
    }
    let mut out = String::new();
    for target in targets {
        let ctx = Ctx {
            index,
            external: query.show_external,
            tests: query.include_tests || is_test(target),
        };
        out.push_str(&label(target));
        out.push('\n');
        let path = &mut vec![key(target)];
        if query.direction != Direction::Incoming {
            out.push_str("outgoing:\n");
            outgoing(&ctx, target, depth, path, &mut out);
        }
        if query.direction != Direction::Outgoing {
            out.push_str("incoming:\n");
            incoming(&ctx, target, depth, path, &mut out);
        }
    }
    Some(out)
}

struct Ctx<'a> {
    index: &'a [Entry],
    external: bool,
    tests: bool,
}

impl Ctx<'_> {
    fn visible(&self, entry: &Entry) -> bool {
        self.tests || !is_test(entry)
    }
}

/// `name (in scope)<tab>file:first..last`, where `scope` is the qualified path of the enclosing
/// definitions (`east::Token`).
fn label(entry: &Entry) -> String {
    let f = &entry.func;
    let scope = if f.scope.is_empty() {
        String::new()
    } else {
        format!(" (in {})", f.scope)
    };
    format!(
        "{}{}\t{}:{}..{}",
        f.name, scope, entry.file, f.first, f.last
    )
}

/// Identity of a definition, used to detect cycles.
fn key(entry: &Entry) -> String {
    format!("{}:{}", entry.file, entry.func.first)
}

/// Whether `def` is in the module named `module`: its file, the folder of a `mod.rs`, or an
/// inline `mod`.
fn in_module(def: &Entry, module: &str) -> bool {
    let path = Path::new(&def.file);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let dir = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("");
    stem == module
        || (matches!(stem, "mod" | "__init__" | "lib" | "main") && dir == module)
        || def
            .func
            .scope
            .innermost()
            .is_some_and(|frame| frame.kind == "mod" && frame.name == module)
}

/// Whether `call`, made in `caller`, can reach the definition `def`.
fn accepts(call: &CallSite, caller: &Entry, def: &Entry) -> bool {
    if def.func.name != call.name {
        return false;
    }
    let owner = def.func.scope.owner();
    match &call.kind {
        CallKind::Any => true,
        CallKind::Free => owner.is_none(),
        CallKind::Method => {
            // `value.f()` with an unknown receiver type can reach any method named `f`.
            owner.is_some() && !(caller.file == def.file && caller.func.first == def.func.first)
        }
        CallKind::MethodOf(receiver) => owner.is_some_and(|o| o.bare_name() == receiver.as_str()),
        CallKind::Own => owner.is_some_and(|o| {
            caller
                .func
                .scope
                .owner()
                .is_some_and(|own| own.kind == "trait" || own.bare_name() == o.bare_name())
        }),
        CallKind::Path(segment) if segment.starts_with(char::is_uppercase) => {
            owner.is_some_and(|o| o.bare_name() == segment.as_str())
        }
        CallKind::Path(module) => owner.is_none() && in_module(def, module),
    }
}

/// Keep same-named methods as uncertain candidates when an inferred receiver type has no
/// matching method in the indexed code.
fn possible_method_target(call: &CallSite, caller: &Entry, def: &Entry) -> bool {
    matches!(&call.kind, CallKind::MethodOf(_))
        && def.func.name == call.name
        && def.func.scope.owner().is_some()
        && !(caller.file == def.file && caller.func.first == def.func.first)
}

/// Return exact targets first. If a typed receiver has no visible exact target, preserve
/// same-named local methods as possible targets instead of reporting the call as external.
fn matching_defs<'a>(ctx: &Ctx<'a>, call: &CallSite, caller: &Entry) -> Vec<&'a Entry> {
    let exact: Vec<&Entry> = ctx
        .index
        .iter()
        .filter(|def| ctx.visible(def) && accepts(call, caller, def))
        .collect();
    if exact.is_empty() && matches!(&call.kind, CallKind::MethodOf(_)) {
        return ctx
            .index
            .iter()
            .filter(|def| ctx.visible(def) && possible_method_target(call, caller, def))
            .collect();
    }
    exact
}

/// A method edge is uncertain when its receiver type is unknown, unmatched, or matches multiple definitions.
fn possible_edge(ctx: &Ctx<'_>, call: &CallSite, caller: &Entry) -> bool {
    match &call.kind {
        CallKind::Method => true,
        CallKind::MethodOf(_) => {
            ctx.index
                .iter()
                .filter(|def| ctx.visible(def) && accepts(call, caller, def))
                .count()
                != 1
        }
        _ => false,
    }
}

/// Whether a file is test code: it is in a `tests` or `test` folder or named like a test.
pub fn is_test_path(file: &str) -> bool {
    let path = Path::new(file);
    let in_tests_dir = path
        .components()
        .any(|c| matches!(c.as_os_str().to_str(), Some("tests" | "test")));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    in_tests_dir
        || matches!(stem, "tests" | "conftest")
        || stem.starts_with("test_")
        || stem.ends_with("_test")
}

/// Whether an entry is test code: a test file or folder, or inside a test module or class.
fn is_test(entry: &Entry) -> bool {
    is_test_path(&entry.file) || entry.func.scope.in_test()
}

fn outgoing(
    ctx: &Ctx<'_>,
    from: &Entry,
    remaining: usize,
    path: &mut Vec<String>,
    out: &mut String,
) {
    let indent = "  ".repeat(path.len());
    let mut hidden: Vec<&str> = Vec::new();
    for call in &from.func.calls {
        let defs = matching_defs(ctx, &call.site, from);
        if defs.is_empty() {
            if ctx.external {
                out.push_str(&format!(
                    "{indent}{}\t(external){}\n",
                    call.site.name,
                    call_lines(&call.lines)
                ));
            } else if !hidden.contains(&call.site.name.as_str()) {
                hidden.push(&call.site.name);
            }
            continue;
        }
        let edge = Edge {
            indent: &indent,
            possible: possible_edge(ctx, &call.site, from),
            lines: &call.lines,
        };
        for def in defs {
            descend(ctx, def, remaining, path, out, &edge, outgoing);
        }
    }
    if !hidden.is_empty() {
        let shown = hidden
            .iter()
            .take(EXTERNAL_SHOWN)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        let more = hidden.len().saturating_sub(EXTERNAL_SHOWN);
        let more = if more > 0 {
            format!(" +{more}")
        } else {
            String::new()
        };
        out.push_str(&format!("{indent}(external: {shown}{more})\n"));
    }
}

fn incoming(ctx: &Ctx<'_>, to: &Entry, remaining: usize, path: &mut Vec<String>, out: &mut String) {
    let indent = "  ".repeat(path.len());
    for caller in ctx.index.iter().filter(|c| ctx.visible(c)) {
        let calls: Vec<&Call> = caller
            .func
            .calls
            .iter()
            .filter(|call| {
                matching_defs(ctx, &call.site, caller)
                    .iter()
                    .any(|def| def.file == to.file && def.func.first == to.func.first)
            })
            .collect();
        if calls.is_empty() {
            continue;
        }
        // Every line of the caller that reaches `to`.
        let mut lines: Vec<usize> = calls
            .iter()
            .flat_map(|call| call.lines.iter().copied())
            .collect();
        lines.sort_unstable();
        lines.dedup();
        let edge = Edge {
            indent: &indent,
            possible: calls
                .iter()
                .all(|call| possible_edge(ctx, &call.site, caller)),
            lines: &lines,
        };
        descend(ctx, caller, remaining, path, out, &edge, incoming);
    }
}

type Step = fn(&Ctx<'_>, &Entry, usize, &mut Vec<String>, &mut String);

/// Print `entry` with the lines of the call that led to it, then follow it one level deeper unless
/// it closes a cycle or the depth is used.
fn descend(
    ctx: &Ctx<'_>,
    entry: &Entry,
    remaining: usize,
    path: &mut Vec<String>,
    out: &mut String,
    edge: &Edge<'_>,
    step: Step,
) {
    let id = key(entry);
    let indent = edge.indent;
    let marker = if edge.possible { "? " } else { "" };
    let lines = call_lines(edge.lines);
    if path.contains(&id) {
        out.push_str(&format!(
            "{indent}{marker}{}{lines}\t(cycle)\n",
            label(entry)
        ));
        return;
    }
    out.push_str(&format!("{indent}{marker}{}{lines}\n", label(entry)));
    if remaining > 1 {
        path.push(id);
        step(ctx, entry, remaining - 1, path, out);
        path.pop();
    }
}

/// How a function was reached: how its row is printed and the call that led to it.
struct Edge<'a> {
    indent: &'a str,
    /// The call may not reach this function (`? `).
    possible: bool,
    /// Lines of the calling function that make the call.
    lines: &'a [usize],
}

/// ` @55,70,81 +2`: the lines of the call, at most [`LINES_SHOWN`] of them and a count of the
/// rest. Empty when there are none.
fn call_lines(lines: &[usize]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let shown = lines
        .iter()
        .take(LINES_SHOWN)
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",");
    match lines.len().saturating_sub(LINES_SHOWN) {
        0 => format!(" @{shown}"),
        more => format!(" @{shown} +{more}"),
    }
}

fn walk(
    node: Node<'_>,
    scope: &mut Lineage,
    current: Option<usize>,
    syntax: &dyn Syntax,
    source: &str,
    imports: &[Alias],
    out: &mut Vec<FnInfo>,
) {
    let mut cur = current;
    let mut entered = false;
    if let Some(def) = syntax.definition(node, source) {
        if syntax.is_function(def.kind) {
            let (first, last) = unit_span(syntax, node, source);
            out.push(FnInfo {
                name: def.name.clone(),
                scope: scope.clone(),
                first,
                last,
                calls: Vec::new(),
            });
            cur = Some(out.len() - 1);
        }
        scope.push(def.kind, &def.name);
        entered = true;
    } else if let (Some(i), Some(mut site)) = (current, syntax.call_site(node, source)) {
        if matches!(site.kind, CallKind::Free | CallKind::Any) {
            site.name = resolve(imports, &site.name).to_string();
        }
        if let CallKind::MethodOf(receiver) = &mut site.kind {
            *receiver = resolve(imports, receiver).to_string();
        }
        let line = node.start_position().row + 1;
        let calls = &mut out[i].calls;
        match calls.iter_mut().find(|call| call.site == site) {
            Some(call) => {
                if !call.lines.contains(&line) {
                    call.lines.push(line);
                }
            }
            None => calls.push(Call {
                site,
                lines: vec![line],
            }),
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, scope, cur, syntax, source, imports, out);
    }
    if entered {
        scope.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langtool::engine::syntax::python::PythonSyntax;
    use crate::langtool::engine::syntax::rust::RustSyntax;

    const RUST: &str = "fn a() { b(); ext(); }\nfn b() { c(); }\nfn c() { a(); }\n";

    const QUALIFIED: &str = concat!(
        "struct A;\n",
        "struct B;\n",
        "impl A {\n",
        "    fn new() -> A { A }\n",
        "    fn go(&self) { Self::new(); self.helper(); B::new(); free(); }\n",
        "    fn helper(&self) {}\n",
        "}\n",
        "impl B {\n",
        "    fn new() -> B { B }\n",
        "}\n",
        "fn free() {}\n",
        "fn new() {}\n",
    );

    fn index(file: &str, syntax: &dyn Syntax, source: &str) -> Vec<Entry> {
        functions(syntax, source)
            .unwrap()
            .into_iter()
            .map(|func| Entry {
                file: file.to_string(),
                func,
            })
            .collect()
    }

    fn query(symbol: &str, direction: Direction, depth: usize) -> Query<'_> {
        Query {
            symbol,
            file: None,
            line: None,
            direction,
            depth,
            show_external: true,
            include_tests: false,
        }
    }

    fn names(calls: &[Call]) -> Vec<&str> {
        calls.iter().map(|c| c.site.name.as_str()).collect()
    }

    #[test]
    fn collects_calls_per_function() {
        let fns = functions(&RustSyntax, RUST).unwrap();
        assert_eq!(fns[0].name, "a");
        assert_eq!(names(&fns[0].calls), ["b", "ext"]);
        assert_eq!(names(&fns[1].calls), ["c"]);
        assert_eq!(fns[0].calls[0].site.kind, CallKind::Free);
    }

    #[test]
    fn edges_name_the_lines_that_make_the_call() {
        let src = "fn a() {\n    b();\n    ext();\n    b();\n    b(); b();\n}\nfn b() {}\nfn c() { a(); }\n";
        let idx = index("x.rs", &RustSyntax, src);
        // Outgoing: every line of `a` that calls it, once each. Incoming: where `c` calls `a`.
        assert_eq!(
            render(&idx, &query("a", Direction::Both, 1)).unwrap(),
            "a\tx.rs:1..6\noutgoing:\n  b\tx.rs:7..7 @2,4,5\n  ext\t(external) @3\nincoming:\n  c\tx.rs:8..8 @8\n"
        );

        // Only the first few lines are named; the rest are counted.
        let many = "fn a() {\n    b();\n    b();\n    b();\n    b();\n    b();\n}\nfn b() {}\n";
        let idx = index("m.rs", &RustSyntax, many);
        assert_eq!(
            render(&idx, &query("a", Direction::Outgoing, 1)).unwrap(),
            "a\tm.rs:1..7\noutgoing:\n  b\tm.rs:8..8 @2,3,4 +2\n"
        );
    }

    #[test]
    fn outgoing_follows_depth_and_marks_external() {
        let idx = index("x.rs", &RustSyntax, RUST);
        let one = render(&idx, &query("a", Direction::Outgoing, 1)).unwrap();
        assert_eq!(
            one,
            "a\tx.rs:1..1\noutgoing:\n  b\tx.rs:2..2 @1\n  ext\t(external) @1\n"
        );
        let two = render(&idx, &query("a", Direction::Outgoing, 2)).unwrap();
        assert_eq!(
            two,
            "a\tx.rs:1..1\noutgoing:\n  b\tx.rs:2..2 @1\n    c\tx.rs:3..3 @2\n  ext\t(external) @1\n"
        );
    }

    #[test]
    fn external_calls_are_summarized_unless_asked_for() {
        let idx = index("x.rs", &RustSyntax, RUST);
        let q = Query {
            show_external: false,
            ..query("a", Direction::Outgoing, 1)
        };
        assert_eq!(
            render(&idx, &q).unwrap(),
            "a\tx.rs:1..1\noutgoing:\n  b\tx.rs:2..2 @1\n  (external: ext)\n"
        );

        let many = "fn f() { a1(); a2(); a3(); a4(); a5(); a6(); a7(); a8(); }\n";
        let idx = index("m.rs", &RustSyntax, many);
        let q = Query {
            show_external: false,
            ..query("f", Direction::Outgoing, 1)
        };
        assert_eq!(
            render(&idx, &q).unwrap(),
            "f\tm.rs:1..1\noutgoing:\n  (external: a1, a2, a3, a4, a5, a6 +2)\n"
        );
    }

    #[test]
    fn incoming_detects_cycles() {
        let idx = index("x.rs", &RustSyntax, RUST);
        let out = render(&idx, &query("a", Direction::Incoming, 3)).unwrap();
        assert_eq!(
            out,
            "a\tx.rs:1..1\nincoming:\n  c\tx.rs:3..3 @3\n    b\tx.rs:2..2 @2\n      a\tx.rs:1..1 @1\t(cycle)\n"
        );
    }

    #[test]
    fn same_named_functions_are_not_cycles() {
        let idx = index("q.rs", &RustSyntax, QUALIFIED);
        let out = render(&idx, &query("go", Direction::Outgoing, 2)).unwrap();
        assert!(!out.contains("(cycle)"), "{out}");
    }

    #[test]
    fn both_directions_and_unknown_symbol() {
        let idx = index("x.rs", &RustSyntax, RUST);
        let out = render(&idx, &query("b", Direction::Both, 1)).unwrap();
        assert_eq!(
            out,
            "b\tx.rs:2..2\noutgoing:\n  c\tx.rs:3..3 @2\nincoming:\n  a\tx.rs:1..1 @1\n"
        );
        assert!(render(&idx, &query("nope", Direction::Both, 1)).is_none());
    }

    #[test]
    fn calls_resolve_by_how_they_were_written() {
        let idx = index("q.rs", &RustSyntax, QUALIFIED);
        let out = render(&idx, &query("go", Direction::Outgoing, 1)).unwrap();
        assert_eq!(
            out,
            concat!(
                "go (in A)\tq.rs:5..5\n",
                "outgoing:\n",
                "  new (in A)\tq.rs:4..4 @5\n",
                "  helper (in A)\tq.rs:6..6 @5\n",
                "  new (in B)\tq.rs:9..9 @5\n",
                "  free\tq.rs:11..11 @5\n",
            )
        );
        // Callers are found the same way: only `go` calls `A::new`, and nothing calls the free `new`.
        let q = Query {
            file: Some("q.rs"),
            line: Some(4),
            ..query("new", Direction::Incoming, 1)
        };
        assert_eq!(
            render(&idx, &q).unwrap(),
            "new (in A)\tq.rs:4..4\nincoming:\n  go (in A)\tq.rs:5..5 @5\n"
        );
        let q = Query {
            file: Some("q.rs"),
            line: Some(12),
            ..query("new", Direction::Incoming, 1)
        };
        assert_eq!(render(&idx, &q).unwrap(), "new\tq.rs:12..12\nincoming:\n");
    }

    #[test]
    fn unknown_receiver_method_edges_are_marked_possible() {
        let src = concat!(
            "struct Store;\n",
            "impl Store { fn put(&self) {} }\n",
            "fn caller(value) { value.put(); }\n",
        );
        let idx = index("calls.rs", &RustSyntax, src);
        let outgoing = render(&idx, &query("caller", Direction::Outgoing, 1)).unwrap();
        assert_eq!(
            outgoing,
            "caller\tcalls.rs:3..3\noutgoing:\n  ? put (in Store)\tcalls.rs:2..2 @3\n"
        );

        let incoming = render(&idx, &query("put", Direction::Incoming, 1)).unwrap();
        assert_eq!(
            incoming,
            "put (in Store)\tcalls.rs:2..2\nincoming:\n  ? caller\tcalls.rs:3..3 @3\n"
        );
    }

    #[test]
    fn module_paths_go_to_free_functions_of_that_module() {
        let user = "fn run() { util::work(); other::work(); }\n";
        let mut idx = index("u.rs", &RustSyntax, user);
        idx.extend(index("util.rs", &RustSyntax, "fn work() {}\n"));
        idx.extend(index("other/mod.rs", &RustSyntax, "fn work() {}\n"));
        let out = render(&idx, &query("run", Direction::Outgoing, 1)).unwrap();
        assert_eq!(
            out,
            "run\tu.rs:1..1\noutgoing:\n  work\tutil.rs:1..1 @1\n  work\tother/mod.rs:1..1 @1\n"
        );
    }

    #[test]
    fn constructors_and_unknown_methods_are_not_functions_of_the_index() {
        let src = "fn f() { Some(1); Ok::<(), ()>(()); v.clone(); }\nfn clone() {}\n";
        let idx = index("x.rs", &RustSyntax, src);
        let fns = functions(&RustSyntax, src).unwrap();
        assert_eq!(names(&fns[0].calls), ["clone"]);
        assert_eq!(fns[0].calls[0].site.kind, CallKind::Method);
        // `v.clone()` is a method call, so the free function `clone` is not its target.
        let q = Query {
            show_external: false,
            ..query("f", Direction::Outgoing, 1)
        };
        assert_eq!(
            render(&idx, &q).unwrap(),
            "f\tx.rs:1..1\noutgoing:\n  (external: clone)\n"
        );
    }

    #[test]
    fn qualified_symbols_pick_the_definition_in_that_type() {
        let idx = index("a.rs", &RustSyntax, QUALIFIED);
        let heads = |symbol: &str| -> Vec<String> {
            render(&idx, &query(symbol, Direction::Incoming, 1))
                .map(|out| {
                    out.lines()
                        .filter(|l| !l.starts_with(' ') && l.contains('\t'))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        assert_eq!(heads("new").len(), 3);
        assert_eq!(heads("B::new"), ["new (in B)\ta.rs:9..9"]);
        assert_eq!(heads("A.new"), ["new (in A)\ta.rs:4..4"]);
        assert!(heads("C::new").is_empty());
    }

    #[test]
    fn nested_scopes_show_as_a_qualified_path() {
        let src = "mod east {\n    impl Token {\n        fn parse(&self) {}\n    }\n}\n";
        let idx = index("q.rs", &RustSyntax, src);
        let out = render(&idx, &query("east::Token::parse", Direction::Outgoing, 1)).unwrap();
        assert_eq!(out, "parse (in east::Token)\tq.rs:3..3\noutgoing:\n");
    }

    #[test]
    fn code_in_any_enclosing_test_module_is_test_code() {
        let src = "fn run() { Fixture::build(); }\nmod tests {\n    impl Fixture {\n        fn build() {}\n    }\n}\n";
        let idx = index("q.rs", &RustSyntax, src);
        let out = render(&idx, &query("run", Direction::Outgoing, 1)).unwrap();
        assert_eq!(out, "run\tq.rs:1..1\noutgoing:\n  build\t(external) @1\n");
        let q = Query {
            include_tests: true,
            ..query("run", Direction::Outgoing, 1)
        };
        assert_eq!(
            render(&idx, &q).unwrap(),
            "run\tq.rs:1..1\noutgoing:\n  build (in tests::Fixture)\tq.rs:4..4 @1\n"
        );
    }

    #[test]
    fn tests_are_hidden_unless_included_or_queried() {
        let mut idx = index("src/a.rs", &RustSyntax, "fn work() {}\n");
        idx.extend(index("tests/t.rs", &RustSyntax, "fn check() { work(); }\n"));
        let out = render(&idx, &query("work", Direction::Incoming, 1)).unwrap();
        assert_eq!(out, "work\tsrc/a.rs:1..1\nincoming:\n");
        let q = Query {
            include_tests: true,
            ..query("work", Direction::Incoming, 1)
        };
        assert_eq!(
            render(&idx, &q).unwrap(),
            "work\tsrc/a.rs:1..1\nincoming:\n  check\ttests/t.rs:1..1 @1\n"
        );
        let out = render(&idx, &query("check", Direction::Outgoing, 1)).unwrap();
        assert_eq!(
            out,
            "check\ttests/t.rs:1..1\noutgoing:\n  work\tsrc/a.rs:1..1 @1\n"
        );
    }

    #[test]
    fn suggests_similar_names() {
        let idx = index(
            "x.rs",
            &RustSyntax,
            "fn create_turn() {}\nfn other() {}\nfn turn() {}\nfn print() {}\n",
        );
        assert_eq!(suggest(&idx, "turn"), ["create_turn", "turn"]);
        // A name that contains a defined name suggests it, the way `workflow_new_turn` hints `turn`.
        assert_eq!(suggest(&idx, "workflow_new_turn"), ["turn"]);
        assert_eq!(suggest(&idx, "pritn"), ["print"]);
        assert!(suggest(&idx, "zzz").is_empty());
    }

    #[test]
    fn calls_through_import_aliases_count_as_calls_of_the_original() {
        let user = "use m::target as t;\nfn run() { t(); }\n";
        let mut idx = index("u.rs", &RustSyntax, user);
        idx.extend(index("d.rs", &RustSyntax, "fn target() {}\n"));
        let out = render(&idx, &query("target", Direction::Incoming, 1)).unwrap();
        assert_eq!(out, "target\td.rs:1..1\nincoming:\n  run\tu.rs:2..2 @2\n");
        let out = render(&idx, &query("run", Direction::Outgoing, 1)).unwrap();
        assert_eq!(out, "run\tu.rs:2..2\noutgoing:\n  target\td.rs:1..1 @2\n");
    }

    #[test]
    fn narrows_by_file_and_line_and_reads_python_methods() {
        let py = "class A:\n    def f(self):\n        g()\n\ndef g():\n    pass\n";
        let mut idx = index("a.py", &PythonSyntax, py);
        idx.extend(index("b.py", &PythonSyntax, "def g():\n    pass\n"));
        let q = Query {
            file: Some("b.py"),
            ..query("g", Direction::Incoming, 1)
        };
        assert_eq!(
            render(&idx, &q).unwrap(),
            "g\tb.py:1..2\nincoming:\n  f (in A)\ta.py:2..3 @3\n"
        );
        let q = Query {
            line: Some(5),
            ..query("g", Direction::Outgoing, 1)
        };
        assert_eq!(render(&idx, &q).unwrap(), "g\ta.py:5..6\noutgoing:\n");
    }

    #[test]
    fn python_self_calls_stay_in_the_class() {
        let py = "class A:\n    def f(self):\n        self.h()\n    def h(self):\n        pass\n\nclass B:\n    def h(self):\n        pass\n";
        let idx = index("a.py", &PythonSyntax, py);
        let out = render(&idx, &query("f", Direction::Outgoing, 1)).unwrap();
        assert_eq!(
            out,
            "f (in A)\ta.py:2..3\noutgoing:\n  h (in A)\ta.py:4..5 @3\n"
        );
    }
}
