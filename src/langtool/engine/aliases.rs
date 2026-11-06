//! Import aliases (`use a::b as c`, `import numpy as np`, `from m import f as g`).
//!
//! Aliases are read per file and matched by name only: no module path or crate resolution. The
//! reference lookup and the call graph use them so that a renamed import still counts as a use
//! of the original name. Only the [`Syntax`] trait is used.

use tree_sitter::Node;

use super::syntax::{Alias, Syntax, parse};

/// All import aliases declared in `source`, in source order. `None` when it cannot be parsed.
pub fn aliases(syntax: &dyn Syntax, source: &str) -> Option<Vec<Alias>> {
    let tree = parse(syntax, source)?;
    let mut out = Vec::new();
    collect(tree.root_node(), syntax, source, &mut out);
    Some(out)
}

/// `symbol` followed by every alias the file gives it: the names that mean `symbol` there.
pub fn names_for(aliases: &[Alias], symbol: &str) -> Vec<String> {
    std::iter::once(symbol.to_string())
        .chain(
            aliases
                .iter()
                .filter(|a| a.original == symbol)
                .map(|a| a.alias.clone()),
        )
        .collect()
}

/// The original name behind `name` when the file imports it under that alias, else `name`.
pub fn resolve<'a>(aliases: &'a [Alias], name: &'a str) -> &'a str {
    aliases
        .iter()
        .find(|a| a.alias == name)
        .map_or(name, |a| a.original.as_str())
}

fn collect(node: Node<'_>, syntax: &dyn Syntax, source: &str, out: &mut Vec<Alias>) {
    if let Some(alias) = syntax.alias(node, source) {
        out.push(alias);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, syntax, source, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langtool::engine::syntax::python::PythonSyntax;
    use crate::langtool::engine::syntax::rust::RustSyntax;

    fn pairs(aliases: &[Alias]) -> Vec<(&str, &str)> {
        aliases
            .iter()
            .map(|a| (a.alias.as_str(), a.original.as_str()))
            .collect()
    }

    #[test]
    fn reads_rust_aliases_including_grouped_and_scoped() {
        let src =
            "use a::b as c;\nuse x::{y as z, w};\nuse std::io::Result as IoResult;\nuse q as _;\n";
        let found = aliases(&RustSyntax, src).unwrap();
        assert_eq!(
            pairs(&found),
            [("c", "b"), ("z", "y"), ("IoResult", "Result")]
        );
    }

    #[test]
    fn reads_python_aliases_for_import_and_from_import() {
        let src = "import numpy as np\nimport os.path as p\nfrom a.b import c as d, e\n";
        let found = aliases(&PythonSyntax, src).unwrap();
        assert_eq!(pairs(&found), [("np", "numpy"), ("p", "path"), ("d", "c")]);
    }

    #[test]
    fn maps_names_both_ways() {
        let found = aliases(&RustSyntax, "use a::b as c;\nuse a::b as d;\n").unwrap();
        assert_eq!(names_for(&found, "b"), ["b", "c", "d"]);
        assert_eq!(names_for(&found, "other"), ["other"]);
        assert_eq!(resolve(&found, "c"), "b");
        assert_eq!(resolve(&found, "b"), "b");
    }
}
