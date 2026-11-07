//! `filesystem_grep`: regex search with context, grouped by file.
//!
//! Output (content mode):
//! ```text
//! src/a.rs
//! 12:    matching line
//! 13-    context line
//! --
//! 40:    another match
//! ```

use std::path::Path;

use regex::{Regex, RegexBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::fs::resolve::Scope;
use crate::common::fs::walk::{Kind, WalkOpts, read_searchable, walk};
use crate::common::params::{PageParams, flex_u64, opt_string_or_vec};
use crate::common::text::{bounded, clip};
use crate::common::{AgentError, Range, Result, ServerConfig};
use crate::workflow::Project;

pub const DESCRIPTION: &str = "Search a file or directory by regex (Rust syntax; `literal` for plain text). `output` is `content`, `files` or `count`. Respects `.gitignore`; skips binary files and files over 8 MiB.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GrepParams {
    /// Regular expression (or literal text with `literal`).
    pub query: String,
    /// File or directory, relative to the project root or absolute. Default `.`.
    #[serde(default)]
    pub implicit_path: Option<String>,
    /// Only search files matching these globs (for example `*.rs`).
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    pub glob: Option<Vec<String>>,
    /// Treat `query` as literal text.
    #[serde(default)]
    pub literal: Option<bool>,
    #[serde(default)]
    pub case_insensitive: Option<bool>,
    /// Lines of context around each match (default 0, max 10).
    #[serde(default, deserialize_with = "flex_u64")]
    pub context_lines: Option<u64>,
    /// `content` (default), `files` (matching paths only) or `count` (matches per file).
    #[serde(default)]
    pub output: Option<GrepOutputMode>,
    /// Include dotfiles and gitignored files.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

const LINE_CLIP: usize = 300;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GrepOutputMode {
    #[default]
    Content,
    Files,
    Count,
}

impl GrepOutputMode {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "content" => Ok(Self::Content),
            "files" => Ok(Self::Files),
            "count" => Ok(Self::Count),
            other => Err(AgentError::invalid(
                "output",
                format!("`{other}`; expected content, files or count"),
            )),
        }
    }
}

impl<'de> Deserialize<'de> for GrepOutputMode {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        GrepOutputMode::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl From<&str> for GrepOutputMode {
    fn from(s: &str) -> Self {
        GrepOutputMode::parse(s).unwrap_or_else(|e| panic!("{e}"))
    }
}

impl From<String> for GrepOutputMode {
    fn from(s: String) -> Self {
        GrepOutputMode::from(s.as_str())
    }
}

pub fn grep(project: Option<&Project>, cfg: &ServerConfig, p: GrepParams) -> Result<String> {
    let scope = Scope::new(
        project.map(Project::root_dir),
        p.implicit_path.as_deref(),
        "implicit_path",
    )?;
    let target = &scope.target;
    let meta = std::fs::symlink_metadata(&target.abs)
        .map_err(|_| AgentError::PathNotFound(target.shown().to_string()))?;

    let pattern = if p.literal.unwrap_or(false) {
        regex::escape(&p.query)
    } else {
        p.query.clone()
    };
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(p.case_insensitive.unwrap_or(false))
        .size_limit(8 << 20)
        .build()
        .map_err(|e| AgentError::invalid("query", e.to_string()))?;
    let mode = p.output.unwrap_or_default();

    let globs = p.glob.clone().unwrap_or_default();
    let show_gitignored = p.show_gitignored.unwrap_or(false);
    let depth = cfg.max_dir_depth.max(1);
    let ctx = bounded(p.context_lines, 0, 0, 10);
    let is_dir = meta.is_dir();
    let run = |offset: usize, limit: usize| -> Result<Search> {
        let mut search = Search {
            re: re.clone(),
            mode,
            ctx,
            offset,
            limit,
            matches_seen: 0,
            files_seen: 0,
            emitted: 0,
            more: false,
            skipped: 0,
            out: String::new(),
        };
        if is_dir {
            let opts = WalkOpts {
                show_gitignored,
                max_depth: depth,
                globs: &globs,
            };
            walk(&scope.root, &target.abs, &opts, |entry| {
                if entry.kind != Kind::File {
                    return true;
                }
                let abs = scope.root.join(&entry.rel);
                search.file(&entry.rel, &abs)
            })?;
        } else {
            search.file(target.shown(), &target.abs);
        }
        Ok(search)
    };

    // `-N..` pages from the end: a counting pass finds the total, then the real pass runs.
    let (offset, limit) = match p.page.tail() {
        None => p.page.window(cfg.default_limit)?,
        Some(n) => {
            let counted = run(usize::MAX, 1)?;
            let total = if mode == GrepOutputMode::Content {
                counted.matches_seen
            } else {
                counted.files_seen
            };
            (total.saturating_sub(n), n)
        }
    };
    let search = run(offset, limit)?;

    let mut out = search.out;
    if out.is_empty() {
        out.push_str("no matches");
        if search.skipped > 0 {
            out.push_str(&format!(
                " ({} binary or oversized files skipped)",
                search.skipped
            ));
        }
        out.push('\n');
    }
    if search.more {
        out.push_str(&format!(
            "[more results; next entries={}]\n",
            Range::next(search.offset + search.emitted, search.limit)
        ));
    }
    Ok(out)
}

struct Search {
    re: Regex,
    mode: GrepOutputMode,
    ctx: usize,
    offset: usize,
    limit: usize,
    matches_seen: usize,
    files_seen: usize,
    emitted: usize,
    more: bool,
    skipped: usize,
    out: String,
}

impl Search {
    /// Search one file. Returns `false` when the page is full and the walk should stop.
    fn file(&mut self, rel: &str, abs: &Path) -> bool {
        let Some(text) = read_searchable(abs) else {
            self.skipped += 1;
            return true;
        };
        let lines: Vec<&str> = text.lines().collect();
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| self.re.is_match(line))
            .map(|(i, _)| i)
            .collect();
        if hits.is_empty() {
            return true;
        }

        match self.mode {
            GrepOutputMode::Files | GrepOutputMode::Count => {
                if self.files_seen >= self.offset {
                    if self.emitted >= self.limit {
                        self.more = true;
                        return false;
                    }
                    if self.mode == GrepOutputMode::Count {
                        self.out.push_str(&format!("{rel}:{}\n", hits.len()));
                    } else {
                        self.out.push_str(rel);
                        self.out.push('\n');
                    }
                    self.emitted += 1;
                }
                self.files_seen += 1;
                true
            }
            GrepOutputMode::Content => {
                let mut picked = Vec::new();
                for &hit in &hits {
                    if self.matches_seen >= self.offset {
                        if self.emitted >= self.limit {
                            self.more = true;
                            break;
                        }
                        picked.push(hit);
                        self.emitted += 1;
                    }
                    self.matches_seen += 1;
                }
                if !picked.is_empty() {
                    render(&mut self.out, rel, &lines, &picked, self.ctx);
                }
                !self.more
            }
        }
    }
}

/// Write matches (`hits`, zero-based and ascending) with `ctx` lines around them.
fn render(out: &mut String, rel: &str, lines: &[&str], hits: &[usize], ctx: usize) {
    out.push_str(rel);
    out.push('\n');
    let mut windows: Vec<(usize, usize)> = Vec::new();
    for &hit in hits {
        let lo = hit.saturating_sub(ctx);
        let hi = (hit + ctx).min(lines.len() - 1);
        match windows.last_mut() {
            Some(last) if lo <= last.1 + 1 => last.1 = last.1.max(hi),
            _ => windows.push((lo, hi)),
        }
    }
    for (n, &(lo, hi)) in windows.iter().enumerate() {
        if n > 0 && ctx > 0 {
            out.push_str("--\n");
        }
        for (i, line) in lines.iter().enumerate().take(hi + 1).skip(lo) {
            let sep = if hits.binary_search(&i).is_ok() {
                ':'
            } else {
                '-'
            };
            out.push_str(&format!("{}{sep}{}\n", i + 1, clip(line, LINE_CLIP)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_windows() {
        let lines = ["a", "b", "c", "d", "e", "f", "g"];
        let mut out = String::new();
        render(&mut out, "f.txt", &lines, &[1, 5], 1);
        assert_eq!(out, "f.txt\n1-a\n2:b\n3-c\n--\n5-e\n6:f\n7-g\n");
    }

    #[test]
    fn merges_adjacent_windows() {
        let lines = ["a", "b", "c", "d"];
        let mut out = String::new();
        render(&mut out, "f", &lines, &[0, 2], 1);
        assert_eq!(out, "f\n1:a\n2-b\n3:c\n4-d\n");
    }
}
