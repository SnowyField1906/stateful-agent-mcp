//! The facts of one file, read from a single parse.
//!
//! The older tools each parse the file for their own question: [`symbols`](crate::engine::symbols)
//! for the definitions, [`index`](crate::engine::index) for the functions and their calls,
//! [`dependencies`](crate::engine::dependencies) for the names a definition uses. [`facts`] answers
//! all of them from one tree, so an overview of many files parses each file once.
//!
//! Names are assigned to the innermost definition that encloses them. A field or an enum variant
//! (a kind listed only on request) is not an owner: the types in a field belong to the struct that
//! declares it.

use std::collections::BTreeSet;
use std::sync::Arc;

use tree_sitter::Node;

use crate::engine::aliases::resolve;
use crate::engine::dependencies::Use;
use crate::engine::index::{Call, canonical_site};
use crate::engine::references::{Role, role};
use crate::engine::scope::{Frame, Lineage, bare_name};
use crate::engine::symbols::Symbol;
use crate::language::{Alias, CallSite, Import, Syntax, header, node_text, parse, unit_span};

/// What one file holds.
#[derive(Clone, Debug)]
pub struct FileFacts {
    /// First line of the documentation of the whole file.
    pub doc: Option<String>,
    /// Number of lines.
    pub lines: usize,
    /// Every definition, in source order, a definition before the ones written inside it.
    pub defs: Vec<Def>,
    /// The text of every identifier outside comments and strings.
    pub idents: BTreeSet<String>,
    /// The imports of the file, which tell where a name that the project does not define comes
    /// from.
    pub imports: Arc<Vec<Import>>,
    pub aliases: Vec<Alias>,
}

/// A definition with what it uses.
#[derive(Clone, Debug)]
pub struct Def {
    /// Its kind, name, scope, lines, signature and documentation. `body` is never set.
    pub symbol: Symbol,
    /// It can be named from outside the file: a way into the code.
    pub exported: bool,
    /// The index in [`FileFacts::defs`] of the definition it is written in.
    pub parent: Option<usize>,
    /// The names in its signature and body that belong to no definition inside it, in order of
    /// first use, without the names it defines itself, imports and fields.
    pub uses: Vec<Use>,
    /// For a function: the distinct calls in its body, in source order. A call inside a nested
    /// function belongs to the nested function only.
    pub calls: Vec<Call>,
}

/// All the facts of `source`. `None` when it cannot be parsed.
pub fn facts(syntax: &dyn Syntax, source: &str) -> Option<FileFacts> {
    let tree = parse(syntax, source)?;
    let root = tree.root_node();
    let mut walk = Walk {
        syntax,
        source,
        scope: Lineage::default(),
        stack: Vec::new(),
        defs: Vec::new(),
        raw: Vec::new(),
        idents: BTreeSet::new(),
        imports: Vec::new(),
        aliases: Vec::new(),
    };
    walk.visit(root);
    let Walk {
        mut defs,
        raw,
        idents,
        imports,
        aliases,
        ..
    } = walk;
    for (def, raw) in defs.iter_mut().zip(raw) {
        def.uses = used_names(&raw, &aliases);
        def.calls = distinct_calls(raw.calls, &aliases);
    }
    Some(FileFacts {
        doc: syntax.file_doc(root, source),
        lines: source.lines().count(),
        defs,
        idents,
        imports: Arc::new(imports),
        aliases,
    })
}

/// What the walk saw below one definition, before the import aliases are known in full.
#[derive(Default)]
struct Raw {
    /// Identifiers as (line, text, role), in source order.
    seen: Vec<(usize, String, Role)>,
    /// The calls made below the definition, to tell how a used name was written.
    sites: Vec<CallSite>,
    /// Names of the definitions written inside it.
    local: BTreeSet<String>,
    /// The calls of a function as (line, site).
    calls: Vec<(usize, CallSite)>,
}

struct Walk<'a> {
    syntax: &'a dyn Syntax,
    source: &'a str,
    scope: Lineage,
    /// Indices of the definitions that enclose the node being visited, outermost first.
    stack: Vec<usize>,
    defs: Vec<Def>,
    raw: Vec<Raw>,
    idents: BTreeSet<String>,
    imports: Vec<Import>,
    aliases: Vec<Alias>,
}

impl Walk<'_> {
    /// The innermost enclosing definition that owns the names written in it.
    fn owner(&self) -> Option<usize> {
        self.stack
            .iter()
            .rev()
            .copied()
            .find(|index| !self.defs[*index].symbol.kind.on_request)
    }

    /// The innermost enclosing function.
    fn function(&self) -> Option<usize> {
        self.stack
            .iter()
            .rev()
            .copied()
            .find(|index| self.defs[*index].symbol.kind.function)
    }

    fn visit(&mut self, node: Node<'_>) {
        let syntax = self.syntax;
        if syntax.text_kinds().contains(&node.kind()) {
            return;
        }
        self.aliases.extend(syntax.alias(node, self.source));
        self.imports.extend(syntax.imports(node, self.source));
        let line = node.start_position().row + 1;

        let mut entered = false;
        if let Some(definition) = syntax.definition(node, self.source) {
            if !definition.kind.on_request
                && let Some(owner) = self.owner()
            {
                self.raw[owner]
                    .local
                    .insert(bare_name(&definition.name).to_string());
            }
            let (first, last) = unit_span(syntax, node, self.source);
            self.stack_push(Def {
                symbol: Symbol {
                    kind: definition.kind,
                    name: definition.name.clone(),
                    trait_name: definition.trait_name.clone(),
                    test: definition.test,
                    scope: self.scope.clone(),
                    first,
                    last,
                    head: line,
                    signature: header(node, self.source),
                    doc: syntax.doc_line(node, self.source).unwrap_or_default(),
                    body: None,
                },
                exported: syntax.exported(node, self.source),
                parent: self.stack.last().copied(),
                uses: Vec::new(),
                calls: Vec::new(),
            });
            self.scope.push(Frame::of(&definition));
            entered = true;
        } else {
            if syntax.identifier(node).is_some() {
                let text = node_text(node, self.source);
                self.idents.insert(text.to_string());
                if let Some(owner) = self.owner() {
                    let how = role(node, syntax, self.source);
                    self.raw[owner].seen.push((line, text.to_string(), how));
                }
            }
            if let Some(site) = syntax.call_site(node, self.source) {
                if let Some(owner) = self.owner() {
                    self.raw[owner].sites.push(site.clone());
                }
                if let Some(function) = self.function() {
                    self.raw[function].calls.push((line, site));
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child);
        }
        if entered {
            self.scope.pop();
            self.stack.pop();
        }
    }

    /// Record `def` and make it the innermost enclosing definition.
    fn stack_push(&mut self, def: Def) {
        self.stack.push(self.defs.len());
        self.defs.push(def);
        self.raw.push(Raw::default());
    }
}

/// The names used in what `raw` saw, as [`Use`]s: the original behind each import alias, one per
/// name and role.
fn used_names(raw: &Raw, aliases: &[Alias]) -> Vec<Use> {
    let defined: BTreeSet<&str> = raw
        .seen
        .iter()
        .filter(|(_, _, how)| *how == Role::Def)
        .map(|(_, text, _)| text.as_str())
        .chain(raw.local.iter().map(String::as_str))
        .collect();
    let mut uses: Vec<Use> = Vec::new();
    for (line, text, how) in &raw.seen {
        if matches!(how, Role::Def | Role::Import | Role::Field) || defined.contains(text.as_str())
        {
            continue;
        }
        let name = resolve(aliases, text);
        match uses
            .iter_mut()
            .find(|used| used.name == name && used.role == *how)
        {
            Some(used) => {
                if used.lines.last() != Some(line) {
                    used.lines.push(*line);
                }
            }
            None => {
                let site = (*how == Role::Call)
                    .then(|| raw.sites.iter().find(|site| site.name == *text))
                    .flatten()
                    .map(|site| {
                        let mut site = site.clone();
                        canonical_site(&mut site, aliases);
                        site
                    });
                uses.push(Use {
                    name: name.to_string(),
                    alias: (name != text.as_str()).then(|| text.clone()),
                    role: *how,
                    lines: vec![*line],
                    site,
                });
            }
        }
    }
    uses
}

/// The distinct calls of `found`, with the lines each is made on and the names that an import
/// renamed given back as they are defined.
fn distinct_calls(found: Vec<(usize, CallSite)>, aliases: &[Alias]) -> Vec<Call> {
    let mut calls: Vec<Call> = Vec::new();
    for (line, mut site) in found {
        canonical_site(&mut site, aliases);
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
    calls
}
