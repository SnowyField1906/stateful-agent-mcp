//! Scan shared by the code intelligence tools that index every supported file below a path
//! (`code_symbols`, `code_references`, `code_calls`): directory walk, per-file indexing, and
//! paging of the results grouped by file.

use std::path::Path;

use crate::config::ServerConfig;
use crate::error::{AgentError, Result};
use crate::lang::{LANGUAGES, Language, Syntax, language_for_path};
use crate::params::PageParams;
use crate::state::Project;

use super::fmt::more_footer;
use super::grep::read_searchable;
use super::resolve::Scope;
use super::walk::{Kind, WalkOpts, walk};

/// Error text for a file whose language has no code intelligence.
pub const UNSUPPORTED: &str =
    "no code intelligence for this file type; supported: rust (.rs), python (.py, .pyi)";

/// Where to scan, as given by a tool's parameters.
pub struct ScanTarget<'a> {
    pub implicit_path: Option<&'a str>,
    /// Depth limit when the path is a directory.
    pub max_depth: usize,
    pub show_gitignored: Option<bool>,
}

/// Items found per file, in walk order, plus the number of files that could not be indexed.
pub struct Scan<T> {
    found: Vec<(String, Vec<T>)>,
    skipped: usize,
}

/// Index the file or every supported file below the target. `index` receives a file's language,
/// syntax and text and returns its items, or `None` when the file cannot be parsed; files without
/// items are left out of the result.
pub fn scan<T>(
    project: Option<&Project>,
    target: &ScanTarget<'_>,
    mut index: impl FnMut(&'static Language, &'static dyn Syntax, &str) -> Option<Vec<T>>,
) -> Result<Scan<T>> {
    let scope = Scope::new(project, target.implicit_path)?;
    let dest = &scope.target;
    let meta = std::fs::symlink_metadata(&dest.abs)
        .map_err(|_| AgentError::PathNotFound(dest.shown().to_string()))?;

    let mut scan = Scan { found: Vec::new(), skipped: 0 };
    if meta.is_dir() {
        let globs: Vec<String> = LANGUAGES
            .iter()
            .filter(|l| l.syntax.is_some())
            .flat_map(|l| l.extensions.iter().map(|e| format!("*.{e}")))
            .collect();
        let opts = WalkOpts {
            show_gitignored: target.show_gitignored.unwrap_or(false),
            max_depth: target.max_depth.max(1),
            globs: &globs,
        };
        walk(&scope.root, &dest.abs, &opts, |entry| {
            if entry.kind == Kind::File {
                scan.add(&entry.rel, &scope.root.join(&entry.rel), &mut index);
            }
            true
        })?;
    } else {
        if language_for_path(dest.shown()).and_then(|l| l.syntax).is_none() {
            return Err(AgentError::invalid("implicit_path", UNSUPPORTED));
        }
        scan.add(dest.shown(), &dest.abs, &mut index);
    }
    Ok(scan)
}

impl<T> Scan<T> {
    fn add<F>(&mut self, label: &str, abs: &Path, index: &mut F)
    where
        F: FnMut(&'static Language, &'static dyn Syntax, &str) -> Option<Vec<T>>,
    {
        let indexed = language_for_path(label)
            .and_then(|l| Some((l, l.syntax?)))
            .zip(read_searchable(abs))
            .and_then(|((lang, syntax), text)| index(lang, syntax, &text));
        match indexed {
            None => self.skipped += 1,
            Some(items) if items.is_empty() => {}
            Some(items) => self.found.push((label.to_string(), items)),
        }
    }

    /// The items per file, and how many files could not be indexed.
    pub fn into_parts(self) -> (Vec<(String, Vec<T>)>, usize) {
        (self.found, self.skipped)
    }

    /// One page of the items, each file's group introduced by its path and rendered by
    /// `render`. `none` is the message when nothing was found.
    pub fn render(
        &self,
        cfg: &ServerConfig,
        page: &PageParams,
        none: &str,
        render: impl Fn(&[T]) -> String,
    ) -> Result<String> {
        let total: usize = self.found.iter().map(|(_, items)| items.len()).sum();
        if total == 0 {
            let mut out = none.to_string();
            if self.skipped > 0 {
                out.push_str(&format!(
                    " ({} unreadable, binary or oversized files skipped)",
                    self.skipped
                ));
            }
            out.push('\n');
            return Ok(out);
        }

        let (offset, limit) = page.window_in(cfg.default_limit, total)?;
        let mut out = String::new();
        let (mut skip, mut left, mut shown) = (offset, limit, 0usize);
        for (label, items) in &self.found {
            if left == 0 {
                break;
            }
            if skip >= items.len() {
                skip -= items.len();
                continue;
            }
            let group = &items[skip..(skip + left).min(items.len())];
            skip = 0;
            left -= group.len();
            shown += group.len();
            out.push_str(label);
            out.push('\n');
            out.push_str(&render(group));
        }
        if let Some(footer) = more_footer(offset, shown, total, limit) {
            out.push_str(&footer);
            out.push('\n');
        }
        Ok(out)
    }
}
