//! Call graph: which functions a function calls and which functions call it.
//!
//! A call is matched to definitions by name and by how it was written (see [`CallKind`]):
//! `Self::new()` and `self.run()` stay in the caller's type, `Type::new()` goes to that type,
//! `module::run()` goes to free functions of that module, and `run()` goes to free functions.
//! Rust `value.run()` calls use an inferred receiver type when available; unknown receivers
//! remain possible method targets marked with `?`, unless the same name is called elsewhere on a
//! receiver that is known to be standard library or a dependency. A receiver typed as something
//! the standard library or a dependency owns (`Vec<_>`, `&str`) has no target but the methods the
//! project itself implements for that type. A receiver typed `&dyn Trait` or
//! `impl Trait` reaches the declaration of the trait and every `impl Trait for T`, also marked `?`. Every edge ends with `@line`, the lines of
//! the calling function that make the call. A call with no definition in the index is placed by
//! [`origins`](super::origins): standard library, dependency or other project code, from the
//! imports of its file and not from a list of names. The functions it links are those of the
//! [`index`](super::index); walking directories, reading files and paginating results belong to
//! [`scan`](super::scan) and [`calltree`](super::calltree).

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::index::{Call, Entry, in_module, is_test, names_a_type};
use super::name_match::ranked_names;
use super::origins::{Origins, Outside, origin_label};
use super::selector::SymbolSelector;
use crate::common::{AgentError, Result};
use crate::language::{CallKind, CallSite};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Incoming,
    Outgoing,
    #[default]
    Both,
}

impl Direction {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "both" => Ok(Self::Both),
            "incoming" => Ok(Self::Incoming),
            "outgoing" => Ok(Self::Outgoing),
            other => Err(AgentError::invalid(
                "direction",
                format!("`{other}`; expected incoming, outgoing or both"),
            )),
        }
    }
}

impl<'de> Deserialize<'de> for Direction {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Direction::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl From<&str> for Direction {
    fn from(s: &str) -> Self {
        Direction::parse(s).unwrap_or_else(|e| panic!("{e}"))
    }
}

impl From<String> for Direction {
    fn from(s: String) -> Self {
        Direction::from(s.as_str())
    }
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
    /// The directory that file names are relative to. The project of a file is the nearest
    /// manifest root between it and this directory (or this directory when there is none).
    /// Imports are looked up below it on disk, and code outside the project or ignored by
    /// `.gitignore` is a dependency.
    pub root: &'a Path,
}

pub const MAX_DEPTH: usize = 3;

/// Lines named after `@` for one call before the rest are counted.
const LINES_SHOWN: usize = 3;

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
    let origins = Origins::new(query.root, index);
    for target in targets {
        let ctx = Ctx {
            index,
            external: query.show_external,
            tests: query.include_tests || is_test(target),
            origins: &origins,
            asked: selector.name,
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
    origins: &'a Origins<'a>,
    /// The name the query asked for: its callers are all shown, whatever the receiver.
    asked: &'a str,
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
        CallKind::Dyn(name) => owner.is_some_and(|o| o.implements(name)),
        CallKind::Own => owner.is_some_and(|o| {
            caller
                .func
                .scope
                .owner()
                .is_some_and(|own| own.kind.interface || own.bare_name() == o.bare_name())
        }),
        CallKind::Path(segment) if names_a_type(&caller.file, segment) => {
            owner.is_some_and(|o| o.bare_name() == segment.as_str())
        }
        CallKind::Path(module) => owner.is_none() && in_module(def, module),
    }
}

/// Whether the language adapter knew the type or trait of the receiver of a method call.
fn typed_receiver(call: &CallSite) -> bool {
    matches!(&call.kind, CallKind::MethodOf(_) | CallKind::Dyn(_))
}

/// Keep same-named methods as uncertain candidates when an inferred receiver type has no
/// matching method in the indexed code.
fn possible_method_target(call: &CallSite, caller: &Entry, def: &Entry) -> bool {
    typed_receiver(call)
        && def.func.name == call.name
        && def.func.scope.owner().is_some()
        && !(caller.file == def.file && caller.func.first == def.func.first)
}

/// Return exact targets first. If a typed receiver has no visible exact target, preserve
/// same-named local methods as possible targets instead of reporting the call as external.
/// A call on a value of unknown type whose name is also called on a standard library or
/// dependency receiver elsewhere has no target unless the query asked for that name.
fn matching_defs<'a>(ctx: &Ctx<'a>, call: &CallSite, caller: &Entry) -> Vec<&'a Entry> {
    if call.name != ctx.asked && ctx.origins.is_outside_method(call) {
        return Vec::new();
    }
    let exact: Vec<&Entry> = ctx
        .index
        .iter()
        .filter(|def| ctx.visible(def) && accepts(call, caller, def))
        .collect();
    // An import or a written path that puts the callee in the standard library or a dependency
    // rules out a same-named function of the project. A typed receiver keeps its exact targets,
    // since a project trait can be implemented for a library type.
    if !exact.is_empty() {
        if typed_receiver(call) {
            return exact;
        }
        return ctx.origins.narrow(call, caller, exact);
    }
    if typed_receiver(call) {
        // The standard library and dependencies own the methods of their types: one that the
        // project implements for such a type is an exact target, and nothing else can be.
        if ctx.origins.is_outside_type(call, caller) {
            return Vec::new();
        }
        let possible: Vec<&Entry> = ctx
            .index
            .iter()
            .filter(|def| ctx.visible(def) && possible_method_target(call, caller, def))
            .collect();
        return ctx.origins.narrow(call, caller, possible);
    }
    exact
}

/// A method edge is uncertain when its receiver type is unknown, unmatched, or matches multiple
/// definitions. A trait receiver is always uncertain: other implementations may exist.
fn possible_edge(ctx: &Ctx<'_>, call: &CallSite, caller: &Entry) -> bool {
    match &call.kind {
        CallKind::Method | CallKind::Dyn(_) => true,
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

fn outgoing(
    ctx: &Ctx<'_>,
    from: &Entry,
    remaining: usize,
    path: &mut Vec<String>,
    out: &mut String,
) {
    let indent = "  ".repeat(path.len());
    let mut outside = Outside::default();
    for call in &from.func.calls {
        let defs = matching_defs(ctx, &call.site, from);
        if defs.is_empty() {
            let verdict = ctx.origins.verdict(&call.site, from);
            if ctx.external {
                out.push_str(&format!(
                    "{indent}{}\t({}){}\n",
                    call.site.name,
                    origin_label(&verdict.origin),
                    call_lines(&call.lines)
                ));
            } else {
                outside.add(&call.site.name, &verdict.origin);
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
    if !outside.is_empty() {
        out.push_str(&format!("{indent}(external: {})\n", outside.render()));
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
