//! Reference lookup: where a name is defined and used, ignoring comments and string literals.
//! Each reference has a role: `def`, `import`, `call`, `type`, `field` or `use`.
//!
//! Only the [`Syntax`] trait is used. Walking directories, reading files and paginating results
//! belong to the tool handler.

use std::collections::BTreeSet;

use tree_sitter::Node;

use super::aliases::{aliases, names_for};
use super::syntax::{Frame, Lineage, SymbolSelector, Syntax, is_test_scope, node_text, parse};

/// How a name is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    Def,
    Import,
    Call,
    Type,
    Field,
    Use,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Def => "def",
            Self::Import => "import",
            Self::Call => "call",
            Self::Type => "type",
            Self::Field => "field",
            Self::Use => "use",
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// 1-based line.
    pub line: usize,
    /// Byte offset of the identifier in the source.
    pub start_byte: usize,
    /// `def` (the name a definition introduces), `import`, `call`, `type`, `field` or `use`.
    pub role: Role,
    /// The import alias written here instead of the symbol, when this occurrence uses one.
    pub alias: Option<String>,
    /// Whether the occurrence is inside test code (`mod tests`, a `Test*` class).
    pub in_test: bool,
    /// The enclosing definitions with a body, outermost first; shown as a qualified path
    /// (`left::caller`).
    pub scope: Lineage,
    /// The source line, trimmed.
    pub text: String,
}

/// Every identifier in `source` whose text equals `symbol` or an import alias the file gives it,
/// in source order. `None` when the source cannot be parsed.
pub fn identifier_names(
    syntax: &dyn Syntax,
    source: &str,
    include_tests: bool,
) -> Option<Vec<String>> {
    let tree = parse(syntax, source)?;
    let mut names = BTreeSet::new();
    collect_names(
        tree.root_node(),
        false,
        syntax,
        source,
        include_tests,
        &mut names,
    );
    Some(names.into_iter().collect())
}

/// Every identifier in `source` whose text equals `symbol` or an import alias the file gives it,
/// in source order. `None` when the source cannot be parsed.
pub fn references(syntax: &dyn Syntax, source: &str, symbol: &str) -> Option<Vec<Reference>> {
    if symbol.is_empty() || !source.contains(symbol) {
        return Some(Vec::new());
    }
    let tree = parse(syntax, source)?;
    let names = names_for(&aliases(syntax, source)?, symbol);
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    collect(
        tree.root_node(),
        false,
        &Lineage::default(),
        &names,
        syntax,
        source,
        &lines,
        &mut out,
    );
    Some(out)
}

/// Narrow `refs`, found by the name of `selector`, to the ones that can belong to its qualified
/// symbol: references inside the qualifying definitions (the `def` itself, and uses in its
/// body or in a method of the type) and uses whose line names the innermost qualifier
/// (`Store::put()`). Receivers are not resolved, so `store.put()` on another line is dropped.
pub fn retain_qualified(refs: &mut Vec<Reference>, selector: &SymbolSelector<'_>) {
    let Some(last) = selector.qualifiers.last() else {
        return;
    };
    let last = last.to_lowercase();
    refs.retain(|r| {
        selector.matches_scope(&r.scope)
            || (r.role != Role::Def && r.text.to_lowercase().contains(&last))
    });
}

/// `line:role:text`, or `line:role(as alias):text` for a use through an import alias, one per line.
pub fn render<'a>(refs: impl IntoIterator<Item = &'a Reference>) -> String {
    render_inner(refs, false)
}

/// Render each reference with its lexical scope when it has one.
pub fn render_with_scope<'a>(refs: impl IntoIterator<Item = &'a Reference>) -> String {
    render_inner(refs, true)
}

fn render_inner<'a>(refs: impl IntoIterator<Item = &'a Reference>, include_scope: bool) -> String {
    let mut out = String::new();
    for r in refs {
        let alias = r
            .alias
            .as_ref()
            .map(|alias| format!("(as {alias})"))
            .unwrap_or_default();
        let scope = if include_scope && !r.scope.is_empty() {
            format!(" (in {})", r.scope)
        } else {
            String::new()
        };
        out.push_str(&format!(
            "{}:{}{}{}:{}\n",
            r.line, r.role, alias, scope, r.text
        ));
    }
    out
}

fn collect_names(
    node: Node<'_>,
    in_test: bool,
    syntax: &dyn Syntax,
    source: &str,
    include_tests: bool,
    out: &mut BTreeSet<String>,
) {
    if syntax.text_kinds().contains(&node.kind()) {
        return;
    }
    let mut in_test = in_test;
    if let Some(def) = syntax.definition(node, source)
        && is_test_scope(def.kind, &def.name)
    {
        in_test = true;
    }
    if (!in_test || include_tests) && syntax.identifier_kinds().contains(&node.kind()) {
        out.insert(node_text(node, source).to_string());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_names(child, in_test, syntax, source, include_tests, out);
    }
}

/// `names[0]` is the symbol itself; the rest are its aliases in this file.
fn collect(
    node: Node<'_>,
    in_test: bool,
    scopes: &Lineage,
    names: &[String],
    syntax: &dyn Syntax,
    source: &str,
    lines: &[&str],
    out: &mut Vec<Reference>,
) {
    if syntax.text_kinds().contains(&node.kind()) {
        return;
    }
    let mut in_test = in_test;
    // The lineage inside this node, when it is a definition with a body.
    let mut entered = None;
    if let Some(def) = syntax.definition(node, source) {
        if is_test_scope(def.kind, &def.name) {
            in_test = true;
        }
        if node.child_by_field_name("body").is_some() {
            entered = Some(scopes.entered(Frame::of(&def)));
        }
    }
    let inner = entered.as_ref().unwrap_or(scopes);
    if syntax.identifier_kinds().contains(&node.kind()) {
        let text = node_text(node, source);
        if let Some(i) = names.iter().position(|n| n == text) {
            let row = node.start_position().row;
            out.push(Reference {
                line: row + 1,
                start_byte: node.start_byte(),
                role: role(node, syntax, source),
                alias: (i > 0).then(|| text.to_string()),
                in_test,
                scope: scopes.clone(),
                text: lines
                    .get(row)
                    .map(|l| l.trim().to_string())
                    .unwrap_or_default(),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_scopes = if is_definition_name(child, syntax, source) {
            scopes
        } else {
            inner
        };
        collect(
            child,
            in_test,
            child_scopes,
            names,
            syntax,
            source,
            lines,
            out,
        );
    }
}

/// How the identifier `node` is used.
pub(super) fn role(node: Node<'_>, syntax: &dyn Syntax, source: &str) -> Role {
    if is_definition_name(node, syntax, source) {
        return Role::Def;
    }
    let mut up = node.parent();
    while let Some(parent) = up {
        if syntax.import_kinds().contains(&parent.kind()) {
            return Role::Import;
        }
        up = parent.parent();
    }
    let text = node_text(node, source);
    let mut cur = node;
    for _ in 0..4 {
        let Some(parent) = cur.parent() else { break };
        let calls_it = syntax.call_name(parent, source).as_deref() == Some(text)
            && parent.child_by_field_name("function").is_some_and(|f| {
                f.start_byte() <= node.start_byte() && node.end_byte() <= f.end_byte()
            });
        if calls_it {
            return Role::Call;
        }
        cur = parent;
    }
    match node.kind() {
        "type_identifier" => Role::Type,
        "field_identifier" => Role::Field,
        _ => Role::Use,
    }
}

/// Whether `node` is the name a definition introduces (`name` field, or `left` for a Python
/// assignment).
fn is_definition_name(node: Node<'_>, syntax: &dyn Syntax, source: &str) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if syntax.definition(parent, source).is_none() {
        return false;
    }
    ["name", "left"].into_iter().any(|field| {
        parent
            .child_by_field_name(field)
            .is_some_and(|n| n.id() == node.id())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langtool::engine::syntax::python::PythonSyntax;
    use crate::langtool::engine::syntax::rust::RustSyntax;

    const RUST: &str = "// helper in a comment\nfn helper() {}\n\nfn main() {\n    let s = \"helper\";\n    helper();\n}\n";
    const PYTHON: &str = "LIMIT = 3\n# LIMIT in a comment\n\ndef f():\n    return LIMIT + 1\n";

    #[test]
    fn identifier_names_ignore_comments_strings_and_hidden_test_scopes() {
        let source = concat!(
            "fn helper() {} // ignored_comment\n",
            "fn main() { let helper_value = \"ignored_string\"; helper(); }\n",
            "mod tests { fn test_only() {} }\n",
        );
        let visible = identifier_names(&RustSyntax, source, false).unwrap();
        assert!(visible.contains(&"helper".into()), "{visible:?}");
        assert!(visible.contains(&"helper_value".into()), "{visible:?}");
        assert!(!visible.contains(&"ignored_comment".into()), "{visible:?}");
        assert!(!visible.contains(&"ignored_string".into()), "{visible:?}");
        assert!(!visible.contains(&"test_only".into()), "{visible:?}");

        let with_tests = identifier_names(&RustSyntax, source, true).unwrap();
        assert!(with_tests.contains(&"test_only".into()), "{with_tests:?}");
    }

    #[test]
    fn identifier_names_include_long_rust_method_identifiers() {
        let source = "impl State { fn validation_action_ran(&self) {} }";
        let names = identifier_names(&RustSyntax, source, false).unwrap();
        assert!(names.contains(&"validation_action_ran".into()), "{names:?}");
    }

    #[test]
    fn finds_rust_definition_and_call_but_not_comment_or_string() {
        let refs = references(&RustSyntax, RUST, "helper").unwrap();
        assert_eq!(render(&refs), "2:def:fn helper() {}\n6:call:helper();\n");
    }

    #[test]
    fn finds_python_assignment_and_use() {
        let refs = references(&PythonSyntax, PYTHON, "LIMIT").unwrap();
        assert_eq!(render(&refs), "1:def:LIMIT = 3\n5:use:return LIMIT + 1\n");
    }

    #[test]
    fn classifies_types_fields_methods_and_arguments() {
        let src = "struct Point { x: i32 }\nfn f(p: Point) {\n    let q = Point { x: 1 };\n    p.x;\n    g(x);\n    p.x();\n}\n";
        let types = references(&RustSyntax, src, "Point").unwrap();
        assert_eq!(
            render(&types),
            "1:def:struct Point { x: i32 }\n2:type:fn f(p: Point) {\n3:type:let q = Point { x: 1 };\n"
        );
        let x = references(&RustSyntax, src, "x").unwrap();
        let roles: Vec<Role> = x.iter().map(|r| r.role).collect();
        assert_eq!(
            roles,
            [Role::Def, Role::Field, Role::Field, Role::Use, Role::Call]
        );
    }

    #[test]
    fn marks_references_in_test_code() {
        let src = "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() { super::a(); }\n}\n";
        let refs = references(&RustSyntax, src, "a").unwrap();
        let flags: Vec<bool> = refs.iter().map(|r| r.in_test).collect();
        assert_eq!(flags, [false, true]);
    }

    #[test]
    fn finds_uses_through_a_rust_import_alias() {
        let src = "use a::helper as h;\nfn main() {\n    h();\n    helper();\n}\n";
        let refs = references(&RustSyntax, src, "helper").unwrap();
        assert_eq!(
            render(&refs),
            "1:import:use a::helper as h;\n1:import(as h):use a::helper as h;\n3:call(as h):h();\n4:call:helper();\n"
        );
    }

    #[test]
    fn finds_uses_through_a_python_import_alias() {
        let src = "from m import f as g\ng()\nf()\n";
        let refs = references(&PythonSyntax, src, "f").unwrap();
        assert_eq!(
            render(&refs),
            "1:import:from m import f as g\n1:import(as g):from m import f as g\n2:call(as g):g()\n3:call:f()\n"
        );
    }

    #[test]
    fn references_carry_their_lexical_scope_as_a_qualified_path() {
        let src = "mod left { fn helper() {} fn caller() { helper(); } }\nmod right { fn helper() {} fn caller() { helper(); } }\n";
        let refs = references(&RustSyntax, src, "helper").unwrap();
        let scopes: Vec<String> = refs.iter().map(|r| r.scope.to_string()).collect();
        assert_eq!(scopes, ["left", "left::caller", "right", "right::caller"]);
        assert_eq!(
            render_with_scope(&refs),
            "1:def (in left):mod left { fn helper() {} fn caller() { helper(); } }\n1:call (in left::caller):mod left { fn helper() {} fn caller() { helper(); } }\n2:def (in right):mod right { fn helper() {} fn caller() { helper(); } }\n2:call (in right::caller):mod right { fn helper() {} fn caller() { helper(); } }\n"
        );
    }

    #[test]
    fn qualified_selectors_keep_references_inside_or_naming_the_qualifier() {
        let src = concat!(
            "impl Store {\n",
            "    fn put(&self) {}\n",
            "    fn bulk(&self) { self.put(); }\n",
            "}\n",
            "impl Queue {\n",
            "    fn put(&self) {}\n",
            "    fn drain(&self) { self.put(); }\n",
            "}\n",
            "fn main() { Store::put(); }\n",
        );
        let selector = SymbolSelector::parse("Store::put");
        let mut refs = references(&RustSyntax, src, selector.name).unwrap();
        assert_eq!(refs.len(), 5);
        retain_qualified(&mut refs, &selector);
        let lines: Vec<usize> = refs.iter().map(|r| r.line).collect();
        assert_eq!(lines, [2, 3, 9]);
    }

    #[test]
    fn missing_symbol_gives_no_references() {
        assert!(references(&RustSyntax, RUST, "nothing").unwrap().is_empty());
        assert!(references(&RustSyntax, RUST, "").unwrap().is_empty());
    }
}
