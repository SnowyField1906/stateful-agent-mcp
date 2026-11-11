//! The function index: every function of the scanned files with the calls it makes.
//!
//! [`origins`](super::origins) places a call that no indexed function answers to, and the
//! [`callgraph`](super::callgraph) links the functions of the index by name. Only the [`Syntax`]
//! trait is used; walking directories and reading files belong to [`scan`](super::scan).

use std::path::Path;
use std::sync::Arc;

use tree_sitter::Node;

use super::aliases::{aliases as file_aliases, resolve};
use super::imports as file_imports;
use super::scope::{Frame, Lineage, bare_name};
use crate::language::{Alias, CallKind, CallSite, Import, Syntax, parse, registry, unit_span};

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
    /// The imports of the file the function is in, shared by all its functions. They tell where a
    /// call that has no definition in the index goes.
    pub imports: Arc<Vec<Import>>,
    /// The types and traits the file declares (`struct`, `enum`, `trait`, `class`, a type alias),
    /// shared by all its functions. A type that a scanned file declares is project code; one that
    /// is only given a trait impl (`impl From<A> for Vec<B>`) is not.
    pub types: Arc<Vec<String>>,
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

/// The index of a scan: one [`Entry`] per function, in scan order, from the functions of each
/// file (see [`scan`](super::scan::scan)).
pub fn entries(files: Vec<(String, Vec<FnInfo>)>) -> Vec<Entry> {
    files
        .into_iter()
        .flat_map(|(file, funcs)| {
            funcs.into_iter().map(move |func| Entry {
                file: file.clone(),
                func,
            })
        })
        .collect()
}

/// All functions in `source`, in source order. `None` when it cannot be parsed.
pub fn functions(syntax: &dyn Syntax, source: &str) -> Option<Vec<FnInfo>> {
    let tree = parse(syntax, source)?;
    let aliases = file_aliases(syntax, source)?;
    let imports = Arc::new(file_imports::collect(tree.root_node(), syntax, source));
    let mut out = Vec::new();
    let mut types = Vec::new();
    walk(
        tree.root_node(),
        &mut Lineage::default(),
        None,
        syntax,
        source,
        &aliases,
        &mut out,
        &mut types,
    );
    let types = Arc::new(types);
    for func in &mut out {
        func.imports = Arc::clone(&imports);
        func.types = Arc::clone(&types);
    }
    Some(out)
}

/// Whether `name` is written like the name of a type in the language of `file`.
pub(super) fn names_a_type(file: &str, name: &str) -> bool {
    registry::for_path(file).is_some_and(|syntax| syntax.is_type_name(name))
}

/// Whether `def` is in the module named `module`: its file, the folder of a module entry file
/// (`mod.rs`, `__init__.py`), or an inline module.
pub(super) fn in_module(def: &Entry, module: &str) -> bool {
    in_module_at(&def.file, &def.func.scope, module)
}

/// Whether a definition written in `file`, inside the definitions of `scope`, is in the module
/// named `module`: [`in_module`] for a definition that is not an [`Entry`].
pub(super) fn in_module_at(file: &str, scope: &Lineage, module: &str) -> bool {
    let path = Path::new(file);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let dir = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("");
    stem == module
        || (dir == module
            && registry::for_path(file)
                .is_some_and(|syntax| syntax.module_entry_stems().contains(&stem)))
        || scope
            .innermost()
            .is_some_and(|frame| frame.kind.module && frame.name == module)
}

/// Whether a file is test code by the conventions of its language.
pub fn is_test_path(file: &str) -> bool {
    registry::for_path(file).is_some_and(|syntax| syntax.is_test_path(file))
}

/// Whether an entry is test code: a test file or folder, or inside a test module or class.
pub(super) fn is_test(entry: &Entry) -> bool {
    is_test_path(&entry.file) || entry.func.scope.in_test()
}

/// `site` with the names that an import renamed given back as they are defined.
pub(super) fn canonical_site(site: &mut CallSite, aliases: &[Alias]) {
    if matches!(site.kind, CallKind::Free | CallKind::Any) {
        site.name = resolve(aliases, &site.name).to_string();
    }
    if let CallKind::MethodOf(receiver) | CallKind::Dyn(receiver) = &mut site.kind {
        *receiver = resolve(aliases, receiver).to_string();
    }
}

fn walk(
    node: Node<'_>,
    scope: &mut Lineage,
    current: Option<usize>,
    syntax: &dyn Syntax,
    source: &str,
    aliases: &[Alias],
    out: &mut Vec<FnInfo>,
    types: &mut Vec<String>,
) {
    let mut cur = current;
    let mut entered = false;
    if let Some(def) = syntax.definition(node, source) {
        if def.kind.declares_type {
            let name = bare_name(&def.name);
            if !types.iter().any(|known| known == name) {
                types.push(name.to_string());
            }
        }
        if def.kind.function {
            let (first, last) = unit_span(syntax, node, source);
            out.push(FnInfo {
                name: def.name.clone(),
                scope: scope.clone(),
                first,
                last,
                calls: Vec::new(),
                imports: Arc::default(),
                types: Arc::default(),
            });
            cur = Some(out.len() - 1);
        }
        scope.push(Frame::of(&def));
        entered = true;
    } else if let (Some(i), Some(mut site)) = (current, syntax.call_site(node, source)) {
        canonical_site(&mut site, aliases);
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
        walk(child, scope, cur, syntax, source, aliases, out, types);
    }
    if entered {
        scope.pop();
    }
}
