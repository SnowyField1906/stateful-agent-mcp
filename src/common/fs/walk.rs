//! Directory traversal and text reading shared by `filesystem_ls`, `filesystem_find` and
//! `filesystem_grep`, and by the code intelligence tools.

use std::fs::Metadata;
use std::path::Path;
use std::time::UNIX_EPOCH;

use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;

use crate::common::{AgentError, Result};

/// Upper bound on collected entries, to keep memory and output bounded.
pub const MAX_ENTRIES: usize = 200_000;

/// Largest file read as searchable text, in bytes.
const MAX_SEARCHABLE_BYTES: u64 = 8 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
}

impl Kind {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "file" | "f" => Ok(Self::File),
            "dir" | "directory" | "d" => Ok(Self::Dir),
            "symlink" | "link" | "l" => Ok(Self::Symlink),
            other => Err(AgentError::invalid(
                "kind",
                format!("`{other}`; expected file, dir or symlink"),
            )),
        }
    }

    pub fn type_char(self) -> char {
        match self {
            Self::File => '-',
            Self::Dir => 'd',
            Self::Symlink => 'l',
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// Project-relative, `/`-separated.
    pub rel: String,
    pub kind: Kind,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: u64,
    pub mode: u32,
}

pub struct WalkOpts<'a> {
    /// Include dotfiles and gitignored entries.
    pub show_gitignored: bool,
    /// Depth below the start directory (1 = its children).
    pub max_depth: usize,
    /// Gitignore-style globs; when present, only matching files are visited.
    pub globs: &'a [String],
}

#[cfg(unix)]
pub fn mode_of(meta: &Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
pub fn mode_of(_: &Metadata) -> u32 {
    0
}

pub fn mtime_of(meta: &Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

fn kind_of(meta: &Metadata) -> Option<Kind> {
    let ty = meta.file_type();
    if ty.is_symlink() {
        Some(Kind::Symlink)
    } else if ty.is_dir() {
        Some(Kind::Dir)
    } else if ty.is_file() {
        Some(Kind::File)
    } else {
        None
    }
}

fn rel_of(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// Entry for a single path (no traversal).
pub fn entry_of(root: &Path, path: &Path) -> Option<Entry> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some(Entry {
        rel: rel_of(root, path)?,
        kind: kind_of(&meta)?,
        size: meta.len(),
        modified: mtime_of(&meta),
        mode: mode_of(&meta),
    })
}

/// Walk below `start`, calling `visit` for each entry until it returns `false`.
/// Order is depth-first with siblings sorted by name. Unreadable entries are skipped.
pub fn walk(
    root: &Path,
    start: &Path,
    opts: &WalkOpts<'_>,
    mut visit: impl FnMut(Entry) -> bool,
) -> Result<()> {
    let mut builder = WalkBuilder::new(start);
    builder
        .standard_filters(!opts.show_gitignored)
        .require_git(false)
        .follow_links(false)
        .max_depth(Some(opts.max_depth))
        .sort_by_file_path(|a, b| a.cmp(b));
    builder.filter_entry(|e| {
        e.depth() == 0 || !matches!(e.file_name().to_str(), Some(".git" | ".AGENTS"))
    });
    if !opts.globs.is_empty() {
        let mut overrides = OverrideBuilder::new(start);
        for glob in opts.globs {
            overrides
                .add(glob)
                .map_err(|e| AgentError::invalid("glob", format!("`{glob}`: {e}")))?;
        }
        builder.overrides(
            overrides
                .build()
                .map_err(|e| AgentError::invalid("glob", e.to_string()))?,
        );
    }

    for result in builder.build() {
        let Ok(dent) = result else { continue };
        if dent.depth() == 0 {
            continue;
        }
        let Ok(meta) = dent.metadata() else { continue };
        let (Some(kind), Some(rel)) = (kind_of(&meta), rel_of(root, dent.path())) else {
            continue;
        };
        let entry = Entry {
            rel,
            kind,
            size: meta.len(),
            modified: mtime_of(&meta),
            mode: mode_of(&meta),
        };
        if !visit(entry) {
            break;
        }
    }
    Ok(())
}

/// All entries below `start`; the flag tells whether [`MAX_ENTRIES`] cut the list.
pub fn collect(root: &Path, start: &Path, opts: &WalkOpts<'_>) -> Result<(Vec<Entry>, bool)> {
    let mut items = Vec::new();
    let mut capped = false;
    walk(root, start, opts, |entry| {
        if items.len() >= MAX_ENTRIES {
            capped = true;
            return false;
        }
        items.push(entry);
        true
    })?;
    Ok((items, capped))
}

fn extension(rel: &str) -> &str {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map_or("", |(_, ext)| ext)
}

/// Sort by `name` (default; keeps tree order unless `dirs_first`), `size` (largest first),
/// `modified` (newest first) or `extension`.
pub fn sort_entries(
    items: &mut [Entry],
    sort_by: Option<&str>,
    reverse: bool,
    dirs_first: bool,
) -> Result<()> {
    let key = sort_by.map(|s| s.trim().to_ascii_lowercase());
    match key.as_deref() {
        None | Some("" | "name") => {
            if dirs_first {
                items.sort_by(|a, b| {
                    (a.kind != Kind::Dir)
                        .cmp(&(b.kind != Kind::Dir))
                        .then_with(|| a.rel.cmp(&b.rel))
                });
            }
        }
        Some("size") => items.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.rel.cmp(&b.rel))),
        Some("modified" | "mtime" | "time") => {
            items.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.rel.cmp(&b.rel)));
        }
        Some("extension" | "ext") => items.sort_by(|a, b| {
            extension(&a.rel)
                .cmp(extension(&b.rel))
                .then_with(|| a.rel.cmp(&b.rel))
        }),
        Some(other) => {
            return Err(AgentError::invalid(
                "sort_by",
                format!("`{other}`; expected name, size, modified or extension"),
            ));
        }
    }
    if reverse {
        items.reverse();
    }
    Ok(())
}

/// UTF-8 text of a reasonably small, non-binary file.
pub fn read_searchable(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_SEARCHABLE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}
