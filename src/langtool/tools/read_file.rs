//! `langtool_read_file`: line ranges of text files, optionally folded (`fold_level`) or limited
//! to the definitions of a symbol, now or as of a past turn. Each file of a request is read on
//! its own: a file that fails is reported in place and does not hide the others.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::common::fs::resolve::{Scope, Target};
use crate::common::params::flex_u64;
use crate::common::range::opt_range;
use crate::common::text::clip;
use crate::common::{AgentError, Range, Result};
use crate::langtool::engine::fold::{self, Row};
use crate::langtool::engine::name_match::ranked_names;
use crate::langtool::engine::scan::unsupported_message;
use crate::langtool::engine::symbols::{self, Symbol};
use crate::langtool::engine::syntax::{SymbolSelector, registry};
use crate::workflow::Project;

pub const DESCRIPTION: &str = "Read 1-32 text files in one call; a file that fails is reported in place and the others are still read. Raw lines are numbered `N<tab>text`; the header shows `[first..last of total]` and `next lines=START..` when more remains. `fold_level` gives a structure view instead: registered Tree-sitter code folds by syntax, Markdown and TOML by heading, other text by indentation. Folded content is one row `A..B<tab>text`, where `A..B` are the hidden source lines to read next. Level 0 is an outline: declarations including methods, doc comments cut to their first line. `symbol` reads one definition by name (`Type::name`, `Class.name` or `mod/Type/name` to qualify) with its docs and attributes (Rust, Python). `at_implicit_id` reads the files as of a past turn.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileParams {
    /// Files to read (1-32), each `{file_path, lines?, numbered?, fold_level?, symbol?}`.
    #[serde(deserialize_with = "files_list")]
    #[schemars(with = "Vec<ReadFileEntry>")]
    pub files: Vec<ReadFileEntry>,
    /// Read the files as they were at the end of this turn or update (for example `01.02`).
    #[serde(default)]
    pub at_implicit_id: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ReadFileEntry {
    /// File, relative to the project root or absolute.
    pub file_path: String,
    /// Source lines, 1-based and inclusive: `10..50`, `10..`, `..50`, `-30..` (last 30). Without
    /// an end at most 400 lines (folded: rows) are shown; with one, at most 2000. Default: from line 1.
    #[serde(default, deserialize_with = "opt_range")]
    #[schemars(with = "Option<String>")]
    pub lines: Option<Range>,
    /// Number each line (default true).
    #[serde(default)]
    pub numbered: Option<bool>,
    /// Fold everything nested deeper than this level (0-16) into rows `A..B<tab>text`: code by
    /// block, Markdown and TOML by heading, other text by indentation. `0` is an outline of
    /// declarations including methods. Omit to read raw lines.
    #[serde(default, deserialize_with = "flex_u64")]
    pub fold_level: Option<u64>,
    /// Read only definitions with this name in a file with a registered Tree-sitter grammar, with their doc comments and
    /// attributes: `name`, `Type::name`, `Class.name` or `mod/Type/name`. Works with `fold_level`, not with `lines`.
    #[serde(default)]
    pub symbol: Option<String>,
}

impl ReadFileEntry {
    /// Whether the request asks for an outline or a symbol rather than raw lines.
    pub fn is_focused(&self) -> bool {
        self.fold_level.is_some() || self.symbol.is_some()
    }
}

/// `files` as a list of file requests. Also accepts a JSON string holding that list, a single
/// request object, or plain path strings; anything else gets a hint about the expected shape.
fn files_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ReadFileEntry>, D::Error> {
    use serde::de::Error;
    const HINT: &str = r#"`files` must be an array of objects, for example [{"file_path": "src/a.rs", "lines": "1..80"}]"#;

    let value = match Value::deserialize(d)? {
        Value::String(raw) => {
            serde_json::from_str::<Value>(&raw).map_err(|_| <D::Error as Error>::custom(HINT))?
        }
        other => other,
    };
    let items = match value {
        Value::Array(items) => items,
        object @ Value::Object(_) => vec![object],
        _ => return Err(Error::custom(HINT)),
    };
    items
        .into_iter()
        .map(|item| match item {
            Value::String(file_path) => Ok(ReadFileEntry {
                file_path,
                ..ReadFileEntry::default()
            }),
            other => {
                serde_json::from_value(other).map_err(|e| Error::custom(format!("{HINT}: {e}")))
            }
        })
        .collect()
}

/// Lines (or folded rows) shown when the request names no end.
const DEFAULT_LINES: usize = 400;
/// Largest explicit window.
const MAX_LINES: usize = 2000;
const MAX_FILE_BYTES: u64 = 16 << 20;
const LINE_CLIP: usize = 500;
const MAX_FOLD_LEVEL: u64 = 16;
const MAX_FILES: usize = 32;
/// Definitions shown for one `symbol`.
const MAX_SYMBOL_HITS: usize = 8;

/// A code file longer than this is worth reading as an outline or by symbol rather than raw.
const UNFOCUSED_READ_LINES: usize = 150;

/// A raw read of a long code file: the request named no lines, fold level or symbol.
#[derive(Debug)]
pub struct UnfocusedRead {
    pub path: String,
    pub lines: usize,
}

/// The text of every file of the request, and the first raw read of a long code file in it.
pub fn read_file(
    project: Option<&Project>,
    p: ReadFileParams,
) -> Result<(String, Option<UnfocusedRead>)> {
    if p.files.is_empty() || p.files.len() > MAX_FILES {
        return Err(AgentError::invalid(
            "files",
            format!("provide 1 to {MAX_FILES} file requests"),
        ));
    }
    let at = p
        .at_implicit_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut blocks = Vec::with_capacity(p.files.len());
    let mut errors = Vec::new();
    let mut unfocused = None;
    for file in &p.files {
        match read_one(project, file, at) {
            Ok((text, raw)) => {
                blocks.push(text);
                unfocused = unfocused.or(raw);
            }
            Err(err) => {
                blocks.push(format!("{}: error: {err}\n", file.file_path.trim()));
                errors.push(err);
            }
        }
    }
    if errors.len() == p.files.len() {
        return Err(match errors.len() {
            1 => errors.remove(0),
            _ => AgentError::invalid("files", format!("none could be read\n{}", blocks.concat())),
        });
    }
    Ok((blocks.join("\n"), unfocused))
}

fn read_one(
    project: Option<&Project>,
    req: &ReadFileEntry,
    at: Option<&str>,
) -> Result<(String, Option<UnfocusedRead>)> {
    let path = req.file_path.trim();
    if path.is_empty() {
        return Err(AgentError::invalid("file_path", "must not be empty"));
    }
    let scope = Scope::new(project.map(Project::root_dir), Some(path), "file_path")?;
    let target = &scope.target;
    if target.is_root() {
        return Err(AgentError::invalid("file_path", "must be a file"));
    }
    let text = match at {
        Some(id) => {
            let project = project.filter(|_| scope.in_project).ok_or_else(|| {
                AgentError::invalid(
                    "at_implicit_id",
                    "applies to project paths; it cannot be combined with paths outside the registered project",
                )
            })?;
            project.read_at(id, &target.rel)?.ok_or_else(|| {
                AgentError::NotFound(format!("{} did not exist at {id}", target.rel))
            })?
        }
        None => read_text(target)?,
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Ok((format!("{} (empty)\n", target.shown()), None));
    }
    let symbol = req
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if symbol.is_some() && req.lines.is_some() {
        return Err(AgentError::invalid(
            "symbol",
            "cannot be combined with `lines`; the symbol selects the lines",
        ));
    }
    let view = View::new(target.shown(), &text, &lines, at, req);
    let unfocused = (symbol.is_none()
        && req.lines.is_none()
        && req.fold_level.is_none()
        && at.is_none()
        && lines.len() > UNFOCUSED_READ_LINES
        && registry::for_path(view.path).is_some())
    .then(|| UnfocusedRead {
        path: view.path.to_string(),
        lines: lines.len(),
    });
    let out = match symbol {
        Some(name) => read_symbol(&view, name)?,
        None => view.render(&window(req.lines, lines.len(), view.path)?, None),
    };
    Ok((out, unfocused))
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
    folded: Option<(fold::Mode, usize, Vec<Row>)>,
}

impl<'a> View<'a> {
    fn new(
        path: &'a str,
        text: &'a str,
        lines: &'a [&'a str],
        at: Option<&'a str>,
        req: &ReadFileEntry,
    ) -> Self {
        let folded = req.fold_level.map(|raw| {
            let level = raw.min(MAX_FOLD_LEVEL) as usize;
            let (mode, rows) = fold::fold_file(path, text, level);
            (mode, level, rows)
        });
        View {
            path,
            text,
            lines,
            at,
            numbered: req.numbered.unwrap_or(true),
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

/// The definitions named `name` (`name`, `Type::name`, `Class.name`) with their doc comments,
/// attributes and decorators.
fn read_symbol(view: &View<'_>, name: &str) -> Result<String> {
    let syntax = registry::for_path(view.path)
        .ok_or_else(|| AgentError::invalid("symbol", unsupported_message()))?;
    let all = symbols::symbols(syntax, view.text)
        .ok_or_else(|| AgentError::invalid("file_path", "could not be parsed"))?;
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
    let mut blocks = Vec::new();
    for hit in hits.iter().take(MAX_SYMBOL_HITS) {
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
        blocks.push(view.render(&w, Some(&format!("{} {}{scope}", hit.kind, hit.name))));
    }
    let mut out = blocks.join("\n");
    if hits.len() > MAX_SYMBOL_HITS {
        out.push_str(&format!(
            "[{} more definitions named `{base}`; qualify the name as `Type::{base}`]\n",
            hits.len() - MAX_SYMBOL_HITS
        ));
    }
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
