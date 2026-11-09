//! Reading one file for `langtool_read_file`: raw lines, a folded outline, or the definitions of
//! some symbols, now or as of a past turn.

use std::collections::BTreeSet;

use super::fold::{self, FoldLevel, Row};
use super::name_match::ranked_names;
use super::scan::unsupported_message;
use super::selector::SymbolSelector;
use super::symbols::{self, Symbol};
use crate::common::fs::resolve::{Scope, Target};
use crate::common::text::clip;
use crate::common::{AgentError, Range, Result, Workspace, root_of};
use crate::language::registry;

/// Lines (or folded rows) shown when the request names no end.
const DEFAULT_LINES: usize = 400;
/// Largest explicit window.
const MAX_LINES: usize = 2000;
const MAX_FILE_BYTES: u64 = 16 << 20;
const LINE_CLIP: usize = 500;
/// Definitions shown for one `symbol`.
const MAX_SYMBOL_HITS: usize = 8;

/// A code file longer than this is worth reading as an outline or by symbol rather than raw.
const UNFOCUSED_READ_LINES: usize = 150;

/// What to read of one file.
pub struct Request<'a> {
    /// Relative to the project root, or absolute.
    pub path: &'a str,
    pub lines: Option<Range>,
    /// Whether each line starts with its number.
    pub numbered: bool,
    pub fold_level: Option<FoldLevel>,
    /// Names of the definitions to read instead of lines (`name`, `Type::name`, `Class.name`).
    pub symbols: Option<Vec<&'a str>>,
    /// The turn, session or update to read the file as of; the file on disk when `None`.
    pub at: Option<&'a str>,
}

/// A raw read of a long code file: the request named no lines, fold level or symbol.
#[derive(Debug)]
pub struct UnfocusedRead {
    pub path: String,
    pub lines: usize,
}

/// What reading one file produced.
pub struct Read {
    pub text: String,
    /// Set when the request read a long code file raw.
    pub unfocused: Option<UnfocusedRead>,
}

/// Read the file of `req`, shown under a header with its line range.
pub fn read(workspace: Option<&dyn Workspace>, req: &Request<'_>) -> Result<Read> {
    let path = req.path.trim();
    if path.is_empty() {
        return Err(AgentError::invalid("file_path", "must not be empty"));
    }
    let scope = Scope::new(root_of(workspace), Some(path), "file_path")?;
    let target = &scope.target;
    if target.is_root() {
        return Err(AgentError::invalid("file_path", "must be a file"));
    }
    let text = match req.at {
        Some(id) => {
            let workspace = workspace.filter(|_| scope.in_project).ok_or_else(|| {
                AgentError::invalid(
                    "at_implicit_id",
                    "applies to project paths; it cannot be combined with paths outside the registered project",
                )
            })?;
            workspace.read_at(id, &target.rel)?.ok_or_else(|| {
                AgentError::NotFound(format!("{} did not exist at {id}", target.rel))
            })?
        }
        None => {
            let text = read_text(target)?;
            if let Some(workspace) = workspace.filter(|_| scope.in_project) {
                workspace.note_seen(&target.rel, &text);
            }
            text
        }
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Ok(Read {
            text: format!("{} (empty)\n", target.shown()),
            unfocused: None,
        });
    }
    let symbols = req.symbols.as_deref().filter(|names| !names.is_empty());
    if symbols.is_some() && req.lines.is_some() {
        return Err(AgentError::invalid(
            "symbol",
            "cannot be combined with `lines`; the symbol selects the lines",
        ));
    }
    let view = View::new(target.shown(), &text, &lines, req);
    let unfocused = (symbols.is_none()
        && req.lines.is_none()
        && req.fold_level.is_none()
        && req.at.is_none()
        && lines.len() > UNFOCUSED_READ_LINES
        && registry::for_path(view.path).is_some())
    .then(|| UnfocusedRead {
        path: view.path.to_string(),
        lines: lines.len(),
    });
    let out = match symbols {
        Some(names) => read_symbols(&view, names)?,
        None => view.render(&window(req.lines, lines.len(), view.path)?, None),
    };
    Ok(Read {
        text: out,
        unfocused,
    })
}

/// The source lines a request selects, and how many lines or rows may be shown of them.
struct Window {
    /// Zero-based first line.
    start: usize,
    /// Zero-based end, exclusive.
    end: usize,
    cap: usize,
}

fn window(range: Option<Range>, total: usize, path: &str) -> Result<Window> {
    let Some(range) = range else {
        return Ok(Window {
            start: 0,
            end: total,
            cap: DEFAULT_LINES,
        });
    };
    if let Some(n) = range.tail() {
        let n = (n as usize).clamp(1, MAX_LINES);
        return Ok(Window {
            start: total.saturating_sub(n),
            end: total,
            cap: n,
        });
    }
    let (start, len) = range.forward(DEFAULT_LINES as u64, MAX_LINES as u64, "lines")?;
    if start >= total {
        return Err(AgentError::invalid(
            "lines",
            format!("{path} has {total} lines"),
        ));
    }
    Ok(match range.end {
        Some(_) => Window {
            start,
            end: (start + len).min(total),
            cap: len,
        },
        None => Window {
            start,
            end: total,
            cap: DEFAULT_LINES,
        },
    })
}

/// One file, read as raw lines or as folded rows.
struct View<'a> {
    path: &'a str,
    text: &'a str,
    lines: &'a [&'a str],
    at: Option<&'a str>,
    numbered: bool,
    crlf: bool,
    /// Rows of the whole file with the mode and level that produced them.
    folded: Option<(fold::Mode, FoldLevel, Vec<Row>)>,
}

impl<'a> View<'a> {
    fn new(path: &'a str, text: &'a str, lines: &'a [&'a str], req: &Request<'a>) -> Self {
        let folded = req.fold_level.map(|lvl| {
            let level = lvl.as_usize();
            let (mode, rows) = fold::fold_file(path, text, level);
            (mode, lvl, rows)
        });
        View {
            path,
            text,
            lines,
            at: req.at,
            numbered: req.numbered,
            crlf: text.contains("\r\n"),
            folded,
        }
    }

    /// The lines or rows of `w` under a header; `label` names the definition of a symbol read.
    fn render(&self, w: &Window, label: Option<&str>) -> String {
        let total = self.lines.len();
        let mut body = String::new();
        let (first, shown_end) = match &self.folded {
            None => {
                let end = w.end.min(w.start + w.cap);
                for (i, line) in self.lines[w.start..end].iter().enumerate() {
                    self.push_line(&mut body, &(w.start + i + 1).to_string(), line);
                }
                (w.start + 1, end)
            }
            Some((_, _, rows)) => {
                let shown: Vec<&Row> = rows
                    .iter()
                    .filter(|r| r.last > w.start && r.first <= w.end)
                    .take(w.cap)
                    .collect();
                for row in &shown {
                    self.push_line(&mut body, &row.locator(), &row.text);
                }
                (
                    shown.first().map_or(w.start + 1, |r| r.first),
                    shown.last().map_or(w.start, |r| r.last),
                )
            }
        };
        let mut out = format!("{} [{first}..{shown_end} of {total}]", self.path);
        if let Some((mode, level, _)) = &self.folded {
            out.push_str(&format!(" fold_level {level} {}", mode.label()));
        }
        if let Some(label) = label {
            out.push(' ');
            out.push_str(label);
        }
        if self.crlf {
            out.push_str(" crlf");
        }
        if let Some(id) = self.at {
            out.push_str(&format!(" @{id}"));
        }
        if shown_end < w.end {
            let next = Range {
                start: shown_end as i64 + 1,
                end: None,
            };
            out.push_str(&format!(" next lines={next}"));
        }
        out.push('\n');
        out.push_str(&body);
        out
    }

    fn push_line(&self, out: &mut String, locator: &str, text: &str) {
        if self.numbered {
            out.push_str(locator);
            out.push('\t');
        }
        out.push_str(&clip(text, LINE_CLIP));
        out.push('\n');
    }
}

/// The definitions named in `names` (`name`, `Type::name`, `Class.name`) with their doc comments,
/// attributes and decorators, rendered in source order.
fn read_symbols(view: &View<'_>, names: &[&str]) -> Result<String> {
    let syntax = registry::for_path(view.path)
        .ok_or_else(|| AgentError::invalid("symbol", unsupported_message()))?;
    let all = symbols::symbols(syntax, view.text)
        .ok_or_else(|| AgentError::invalid("file_path", "could not be parsed"))?;

    let mut seen = BTreeSet::new();
    let mut out_blocks = Vec::new();
    for &name in names {
        let selector = SymbolSelector::parse(name);
        let base = selector.name;
        let hits: Vec<&Symbol> = all
            .iter()
            .filter(|s| s.name == base && selector.matches_scope(&s.scope))
            .collect();
        if hits.is_empty() {
            return Err(AgentError::NotFound(no_symbol(
                &all, view.path, name, &selector,
            )));
        }
        for hit in hits.iter().take(MAX_SYMBOL_HITS) {
            if !seen.insert((hit.first, hit.last)) {
                continue;
            }
            let w = Window {
                start: hit.first - 1,
                end: hit.last,
                cap: MAX_LINES,
            };
            let scope = if hit.scope.is_empty() {
                String::new()
            } else {
                format!(" (in {})", hit.scope)
            };
            out_blocks.push((
                hit.first,
                view.render(&w, Some(&format!("{} {}{scope}", hit.kind, hit.name))),
            ));
        }
        if hits.len() > MAX_SYMBOL_HITS {
            out_blocks.push((
                usize::MAX,
                format!(
                    "[{} more definitions named `{base}`; qualify the name as `Type::{base}`]\n",
                    hits.len() - MAX_SYMBOL_HITS
                ),
            ));
        }
    }
    out_blocks.sort_by_key(|(first, _)| *first);
    let out = out_blocks
        .into_iter()
        .map(|(_, block)| block)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(out)
}

fn no_symbol(all: &[Symbol], path: &str, name: &str, selector: &SymbolSelector<'_>) -> String {
    let near = ranked_names(
        all.iter()
            .filter(|symbol| selector.matches_scope(&symbol.scope))
            .map(|symbol| symbol.name.clone()),
        selector.name,
        5,
        true,
    );
    let mut message = format!("no definition `{name}` in {path}");
    if !near.is_empty() {
        let near = near
            .into_iter()
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
            .join(", ");
        message.push_str(&format!("; similar names: {near}"));
    }
    message
}

fn read_text(target: &Target) -> Result<String> {
    let meta = std::fs::metadata(&target.abs).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AgentError::PathNotFound(target.shown().to_string()),
        _ => AgentError::io_at(&target.abs, e),
    })?;
    if meta.is_dir() {
        return Err(AgentError::invalid(
            &target.rel,
            "is a directory; use filesystem_ls",
        ));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(AgentError::invalid(
            &target.rel,
            format!(
                "{} bytes; files over {MAX_FILE_BYTES} bytes are not read",
                meta.len()
            ),
        ));
    }
    let bytes = std::fs::read(&target.abs).map_err(|e| AgentError::io_at(&target.abs, e))?;
    String::from_utf8(bytes)
        .map_err(|_| AgentError::invalid(&target.rel, "not valid UTF-8 text (binary file?)"))
}
