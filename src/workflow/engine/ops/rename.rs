//! `RenameSymbol`: renames the identifiers of a symbol in the files below a path.
//!
//! Matching is by name, like `langtool_find_references`, because Tree-sitter does not resolve
//! types. A bare name renames every identifier with that text. A qualified symbol (`Store::put`)
//! keeps to identifiers inside `Store` and to lines that name it, so `store.put()` elsewhere is
//! not renamed; the summary counts the identifiers left alone. Comments and string literals are
//! never touched, and a use through an import alias keeps the alias. The rename works on the
//! overlay, so it sees the files that earlier operations of the batch created or changed, and the
//! turn ledger records the resulting hunks.

use std::collections::BTreeSet;
use std::path::Path;

use crate::common::fs::walk::{Kind, WalkOpts, walk};
use crate::common::pathspec::matches_pathspec;
use crate::common::{AgentError, Result};
use crate::langtool::engine::references;
use crate::langtool::engine::syntax::{SymbolSelector, Syntax, registry};

use super::overlay::Overlay;
use super::symbol_edit::has_syntax_errors;

/// Deepest directory level searched below the path.
const MAX_DEPTH: usize = 64;

/// What renaming did to the text of one file.
#[derive(Debug, PartialEq, Eq)]
pub struct FileRename {
    pub text: String,
    /// Identifiers renamed.
    pub renamed: usize,
    /// Same-named identifiers left alone because they are outside the qualified symbol.
    pub left: usize,
    /// Identifiers that already have the new name, which the renamed ones can clash with.
    pub existing: usize,
}

/// Totals over all files of one rename.
#[derive(Debug, Default)]
struct Tally {
    files: usize,
    renamed: usize,
    left: usize,
    existing: usize,
    skipped: usize,
}

/// Rename the identifiers of `selector` in `text`, the content of a file parsed by `syntax`.
/// Fails when the rename would leave a syntax error in a file that had none.
pub fn rename_in_text(
    syntax: &dyn Syntax,
    text: &str,
    selector: &SymbolSelector<'_>,
    new_name: &str,
) -> std::result::Result<FileRename, String> {
    let mut refs = references::references(syntax, text, selector.name)
        .ok_or_else(|| "cannot be parsed".to_string())?;
    // A use through an import alias is written with the alias, which keeps its name.
    refs.retain(|r| r.alias.is_none());
    let same_name = refs.len();
    references::retain_qualified(&mut refs, selector);
    if refs.is_empty() {
        return Ok(FileRename {
            text: text.to_string(),
            renamed: 0,
            left: same_name,
            existing: 0,
        });
    }

    let mut out = String::with_capacity(text.len() + refs.len() * new_name.len());
    let mut at = 0;
    for r in &refs {
        out.push_str(&text[at..r.start_byte]);
        out.push_str(new_name);
        at = r.start_byte + selector.name.len();
    }
    out.push_str(&text[at..]);

    if has_syntax_errors(syntax, &out) && !has_syntax_errors(syntax, text) {
        return Err(format!(
            "renaming to `{new_name}` leaves a syntax error; is it a valid name here?"
        ));
    }
    let existing = references::references(syntax, text, new_name).map_or(0, |found| {
        found.iter().filter(|r| r.alias.is_none()).count()
    });
    Ok(FileRename {
        text: out,
        renamed: refs.len(),
        left: same_name - refs.len(),
        existing,
    })
}

/// Rename `symbol` to `new_name` in the supported files of `scope` (a file or directory; the
/// whole project when `None`), in the overlay. Returns the summary line of the operation.
pub fn rename_symbol(
    ov: &mut Overlay,
    root: &Path,
    symbol: &str,
    new_name: &str,
    scope: Option<&str>,
) -> Result<String> {
    let selector = SymbolSelector::parse(symbol);
    if selector.name.is_empty() {
        return Err(AgentError::invalid("symbol", "must not be empty"));
    }
    check_new_name(new_name)?;
    if new_name == selector.name {
        return Err(AgentError::invalid(
            "new_name",
            "is already the name of the symbol",
        ));
    }

    let mut tally = Tally::default();
    for file in candidate_files(ov, root, scope)? {
        let Some(syntax) = registry::for_path(&file) else {
            continue;
        };
        let text = match ov.get(&file) {
            Ok(Some(text)) => text,
            Ok(None) => continue,
            Err(_) => {
                tally.skipped += 1;
                continue;
            }
        };
        let rename = rename_in_text(syntax, &text, &selector, new_name)
            .map_err(|reason| AgentError::invalid(&file, reason))?;
        tally.left += rename.left;
        if rename.renamed > 0 {
            tally.files += 1;
            tally.renamed += rename.renamed;
            tally.existing += rename.existing;
            ov.set(&file, Some(rename.text))?;
        }
    }

    if tally.renamed == 0 {
        return Err(AgentError::invalid(
            "symbol",
            nothing_to_rename(symbol, selector.name, scope, tally.left),
        ));
    }
    Ok(summary(symbol, new_name, scope, &tally))
}

/// A new name must be a plain identifier of the languages that are renamed: letters, digits and
/// `_`, not starting with a digit.
fn check_new_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_');
    if valid {
        Ok(())
    } else {
        Err(AgentError::invalid(
            "new_name",
            "must be a plain identifier: letters, digits and `_`, not starting with a digit",
        ))
    }
}

/// Supported files below `scope` that exist on disk (gitignored ones excluded) or were created
/// or changed by earlier operations of the batch.
fn candidate_files(ov: &Overlay, root: &Path, scope: Option<&str>) -> Result<Vec<String>> {
    let mut files = BTreeSet::new();
    let start = scope.map_or_else(|| root.to_path_buf(), |p| root.join(p));
    match scope {
        Some(p) if start.is_file() => {
            files.insert(p.to_string());
        }
        _ if start.is_dir() => {
            let globs: Vec<String> = registry::supported_languages()
                .flat_map(|language| language.extensions.iter().map(|ext| format!("*.{ext}")))
                .collect();
            let opts = WalkOpts {
                show_gitignored: false,
                max_depth: MAX_DEPTH,
                globs: &globs,
            };
            walk(root, &start, &opts, |entry| {
                if entry.kind == Kind::File {
                    files.insert(entry.rel);
                }
                true
            })?;
        }
        _ => {}
    }
    for (rel, entry) in ov.changed() {
        if entry.current.is_some() && scope.is_none_or(|p| matches_pathspec(rel, p)) {
            files.insert(rel.clone());
        }
    }
    if files.is_empty()
        && let Some(p) = scope
        && !start.exists()
    {
        return Err(AgentError::PathNotFound(p.to_string()));
    }
    Ok(files.into_iter().collect())
}

fn nothing_to_rename(symbol: &str, name: &str, scope: Option<&str>, left: usize) -> String {
    let place = scope.map_or(String::new(), |p| format!(" below `{p}`"));
    if left > 0 {
        format!(
            "no identifier of `{symbol}`{place}; {} identifiers named `{name}` are outside it",
            left
        )
    } else {
        format!("no identifier named `{name}` found{place}")
    }
}

fn summary(symbol: &str, new_name: &str, scope: Option<&str>, tally: &Tally) -> String {
    let place = scope.map_or(String::new(), |p| format!(" in {p}"));
    let mut out = format!(
        "RenameSymbol {symbol} → {new_name}{place}: {} in {}",
        count(tally.renamed, "identifier"),
        count(tally.files, "file")
    );
    if tally.left > 0 {
        out.push_str(&format!(
            "; left {} outside `{symbol}`",
            count(tally.left, "same-named identifier")
        ));
    }
    if tally.existing > 0 {
        out.push_str(&format!(
            "; `{new_name}` already names {} in these files, check for clashes",
            count(tally.existing, "identifier")
        ));
    }
    if tally.skipped > 0 {
        out.push_str(&format!(
            "; skipped {} that could not be read",
            count(tally.skipped, "file")
        ));
    }
    out
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langtool::engine::syntax::python::PythonSyntax;
    use crate::langtool::engine::syntax::rust::RustSyntax;

    fn rename(
        syntax: &dyn Syntax,
        text: &str,
        symbol: &str,
        new_name: &str,
    ) -> std::result::Result<FileRename, String> {
        rename_in_text(syntax, text, &SymbolSelector::parse(symbol), new_name)
    }

    #[test]
    fn renames_identifiers_but_not_comments_strings_or_import_aliases() {
        let src = concat!(
            "// helper in a comment\n",
            "use a::helper as h;\n",
            "fn helper() {}\n",
            "\n",
            "fn main() {\n",
            "    let s = \"helper\";\n",
            "    helper();\n",
            "    h();\n",
            "}\n",
        );
        let out = rename(&RustSyntax, src, "helper", "assist").unwrap();
        assert_eq!(out.renamed, 3);
        assert_eq!((out.left, out.existing), (0, 0));
        assert_eq!(
            out.text,
            concat!(
                "// helper in a comment\n",
                "use a::assist as h;\n",
                "fn assist() {}\n",
                "\n",
                "fn main() {\n",
                "    let s = \"helper\";\n",
                "    assist();\n",
                "    h();\n",
                "}\n",
            )
        );
    }

    #[test]
    fn a_qualified_symbol_keeps_to_its_type_and_counts_what_it_leaves() {
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
        let out = rename(&RustSyntax, src, "Store::put", "insert").unwrap();
        assert_eq!((out.renamed, out.left), (3, 2));
        assert_eq!(
            out.text,
            concat!(
                "impl Store {\n",
                "    fn insert(&self) {}\n",
                "    fn bulk(&self) { self.insert(); }\n",
                "}\n",
                "impl Queue {\n",
                "    fn put(&self) {}\n",
                "    fn drain(&self) { self.put(); }\n",
                "}\n",
                "fn main() { Store::insert(); }\n",
            )
        );
    }

    #[test]
    fn renames_python_names_but_not_comments() {
        let src = "LIMIT = 3\n# LIMIT in a comment\n\ndef f():\n    return LIMIT + 1\n";
        let out = rename(&PythonSyntax, src, "LIMIT", "CAP").unwrap();
        assert_eq!(out.renamed, 2);
        assert_eq!(
            out.text,
            "CAP = 3\n# LIMIT in a comment\n\ndef f():\n    return CAP + 1\n"
        );
    }

    #[test]
    fn a_rename_that_breaks_the_syntax_is_rejected() {
        // `if = 1` cannot be parsed; Tree-sitter's Rust grammar would accept a keyword as a name.
        let src = "x = 1\nprint(x)\n";
        assert!(!has_syntax_errors(&PythonSyntax, src));
        let err = rename(&PythonSyntax, src, "x", "if").unwrap_err();
        assert!(err.contains("syntax error"), "{err}");
    }

    #[test]
    fn existing_uses_of_the_new_name_are_counted_as_possible_clashes() {
        let src = "fn a() {}\nfn b() {}\nfn main() { a(); b(); }\n";
        let out = rename(&RustSyntax, src, "a", "b").unwrap();
        assert_eq!((out.renamed, out.existing), (2, 2));
    }

    #[test]
    fn new_names_are_plain_identifiers() {
        for ok in ["assist", "_x1", "Größe"] {
            assert!(check_new_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "1x", "a-b", "a::b", "a b"] {
            assert!(check_new_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn summary_mentions_only_what_happened() {
        let tally = Tally {
            files: 1,
            renamed: 1,
            ..Tally::default()
        };
        assert_eq!(
            summary("put", "insert", None, &tally),
            "RenameSymbol put → insert: 1 identifier in 1 file"
        );
        let tally = Tally {
            files: 2,
            renamed: 5,
            left: 2,
            existing: 1,
            skipped: 1,
        };
        assert_eq!(
            summary("Store::put", "insert", Some("src"), &tally),
            "RenameSymbol Store::put → insert in src: 5 identifiers in 2 files; left 2 same-named identifiers outside `Store::put`; `insert` already names 1 identifier in these files, check for clashes; skipped 1 file that could not be read"
        );
    }
}
