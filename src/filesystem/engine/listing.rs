//! The listing engine behind `filesystem_ls` and `filesystem_find`.

use crate::common::fs::resolve::Scope;
use crate::common::fs::walk::{Entry, Kind, SortBy, WalkOpts, collect, entry_of, sort_entries};
use crate::common::text::{human_size, more_footer, perms};
use crate::common::{AgentError, PageParams, Range, Result};

/// A normalized listing request.
pub(crate) struct ListSpec {
    /// Depth below the path (1 = children only).
    pub depth: usize,
    pub show_gitignored: bool,
    pub globs: Vec<String>,
    pub kind: Option<Kind>,
    pub size: Option<Range>,
    pub sort_by: Option<SortBy>,
    pub reverse: bool,
    pub long: bool,
    pub dirs_first: bool,
    /// Show names relative to the listed directory instead of the project root.
    pub relative_names: bool,
    pub page: PageParams,
    pub default_limit: usize,
}

pub(crate) fn list(scope: &Scope, spec: ListSpec) -> Result<String> {
    let target = &scope.target;
    let meta = std::fs::symlink_metadata(&target.abs)
        .map_err(|_| AgentError::PathNotFound(target.shown().to_string()))?;
    let is_dir = meta.is_dir() || (!scope.in_project && target.abs.is_dir());

    let (mut items, capped) = if is_dir {
        let opts = WalkOpts {
            show_gitignored: spec.show_gitignored,
            max_depth: spec.depth,
            globs: &spec.globs,
        };
        collect(&scope.root, &target.abs, &opts)?
    } else {
        let entry = entry_of(&scope.root, &target.abs)
            .ok_or_else(|| AgentError::PathNotFound(target.shown().to_string()))?;
        (vec![entry], false)
    };

    items.retain(|e| {
        spec.kind.is_none_or(|k| e.kind == k)
            && spec.size.is_none_or(|r| {
                e.kind == Kind::File
                    && r.first().is_none_or(|m| e.size >= m)
                    && r.end.is_none_or(|m| e.size <= m)
            })
    });
    sort_entries(&mut items, spec.sort_by, spec.reverse, spec.dirs_first)?;

    let total = items.len();
    let (offset, limit) = spec.page.window_in(spec.default_limit, total)?;
    let page: Vec<&Entry> = items.iter().skip(offset).take(limit).collect();
    let strip =
        (is_dir && spec.relative_names && !target.is_root()).then(|| format!("{}/", target.rel));

    let mut out = String::new();
    if is_dir {
        if total == 0 {
            out.push_str(&format!("{}/ (empty)\n", target.shown()));
        } else {
            out.push_str(&format!("{}/ ({total})\n", target.shown()));
        }
    }
    for entry in &page {
        out.push_str(&line(entry, spec.long, strip.as_deref()));
        out.push('\n');
    }
    if let Some(footer) = more_footer(offset, page.len(), total, limit) {
        out.push_str(&footer);
        out.push('\n');
    } else if offset > 0 && page.is_empty() {
        out.push_str(&format!(
            "[entries {}.. is past the {total} results]\n",
            offset + 1
        ));
    }
    if capped {
        out.push_str("[listing capped; narrow with `folder_path` or `glob`]\n");
    }
    Ok(out)
}

fn line(entry: &Entry, long: bool, strip: Option<&str>) -> String {
    let shown = strip
        .and_then(|prefix| entry.rel.strip_prefix(prefix))
        .unwrap_or(&entry.rel);
    let name = match entry.kind {
        Kind::Dir => format!("{shown}/"),
        Kind::Symlink => format!("{shown}@"),
        Kind::File => shown.to_string(),
    };
    if long {
        format!(
            "{} {:>9} {} {name}",
            perms(entry.mode, entry.kind.type_char()),
            entry.size,
            entry.modified
        )
    } else if entry.kind == Kind::File {
        format!("{name}  {}", human_size(entry.size))
    } else {
        name
    }
}
