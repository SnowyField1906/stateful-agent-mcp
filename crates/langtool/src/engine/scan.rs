//! Scan shared by the code intelligence tools that index every supported file below a path
//! (`langtool_find_symbols`, `langtool_find_references`, `langtool_view_callgraph`): directory walk, per-file indexing, and
//! paging of the results grouped by file.

use std::path::{Path, PathBuf};

use crate::common::fs::resolve::Scope;
use crate::common::fs::walk::{Kind, WalkOpts, read_searchable, walk};
use crate::common::text::more_footer;
use crate::common::{AgentError, PageParams, Result, ServerConfig, Workspace, root_of};
use crate::language::{Language, Syntax, language_for_path, registry};

/// Error text for a file whose language has no code intelligence.
pub fn unsupported_message() -> String {
    format!(
        "no code intelligence for this file type; supported: {}",
        registry::supported_file_types()
    )
}

/// Where to scan, as given by a tool's parameters.
pub struct ScanTarget<'a> {
    pub path: Option<&'a str>,
    /// Depth limit when the path is a directory.
    pub max_depth: usize,
    pub show_gitignored: Option<bool>,
}

/// Items found per file, in walk order, plus the number of files that could not be indexed.
pub struct Scan<T> {
    found: Vec<(String, Vec<T>)>,
    skipped: usize,
}

/// The supported files that the target names or holds, as (shown path, absolute path), in walk
/// order.
pub fn files(
    workspace: Option<&dyn Workspace>,
    target: &ScanTarget<'_>,
) -> Result<Vec<(String, PathBuf)>> {
    let scope = Scope::new(root_of(workspace), target.path, "implicit_path")?;
    let dest = &scope.target;
    let meta = std::fs::symlink_metadata(&dest.abs)
        .map_err(|_| AgentError::PathNotFound(dest.shown().to_string()))?;

    let mut found = Vec::new();
    if meta.is_dir() {
        let globs: Vec<String> = registry::supported_languages()
            .flat_map(|language| language.extensions.iter().map(|ext| format!("*.{ext}")))
            .collect();
        let opts = WalkOpts {
            show_gitignored: target.show_gitignored.unwrap_or(false),
            max_depth: target.max_depth.max(1),
            globs: &globs,
        };
        walk(&scope.root, &dest.abs, &opts, |entry| {
            if entry.kind == Kind::File {
                let abs = scope.root.join(&entry.rel);
                found.push((entry.rel, abs));
            }
            true
        })?;
    } else {
        if registry::for_path(dest.shown()).is_none() {
            return Err(AgentError::invalid("implicit_path", unsupported_message()));
        }
        found.push((dest.shown().to_string(), dest.abs.clone()));
    }
    Ok(found)
}

/// Index the file or every supported file below the target. `index` receives a file's language,
/// syntax, text and label (its shown path) and returns its items, or `None` when the file cannot
/// be parsed; files without
/// items are left out of the result.
pub fn scan<T>(
    workspace: Option<&dyn Workspace>,
    target: &ScanTarget<'_>,
    mut index: impl FnMut(&'static Language, &'static dyn Syntax, &str, &str) -> Option<Vec<T>>,
) -> Result<Scan<T>> {
    let mut scan = Scan {
        found: Vec::new(),
        skipped: 0,
    };
    for (label, abs) in files(workspace, target)? {
        scan.add(&label, &abs, &mut index);
    }
    Ok(scan)
}

impl<T> Scan<T> {
    fn add<F>(&mut self, label: &str, abs: &Path, index: &mut F)
    where
        F: FnMut(&'static Language, &'static dyn Syntax, &str, &str) -> Option<Vec<T>>,
    {
        let indexed = language_for_path(label)
            .and_then(|l| Some((l, registry::of(l)?)))
            .zip(read_searchable(abs))
            .and_then(|((lang, syntax), text)| index(lang, syntax, &text, label));
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
