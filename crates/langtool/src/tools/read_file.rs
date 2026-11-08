//! `langtool_read_file`: line ranges of text files, optionally folded (`fold_level`) or limited
//! to the definitions of a symbol, now or as of a past turn. Each file of a request is read on
//! its own: a file that fails is reported in place and does not hide the others.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::common::params::Names;
use crate::common::range::opt_range;
use crate::common::{AgentError, Range, Result, Workspace};
use crate::engine::fold::FoldLevel;
use crate::engine::reader::{self, Request, UnfocusedRead};

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
    #[serde(default)]
    pub fold_level: Option<FoldLevel>,
    /// Read only definitions with this name in a file with a registered Tree-sitter grammar, with their doc comments and
    /// attributes: `name`, `Type::name`, `Class.name` or `mod/Type/name`. Works with `fold_level`, not with `lines`.
    #[serde(default)]
    #[schemars(with = "Option<Vec<String>>")]
    pub symbol: Option<Names>,
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

/// Most files one request may name.
const MAX_FILES: usize = 32;

/// The text of every file of the request, and the first raw read of a long code file in it.
pub fn read_file(
    workspace: Option<&dyn Workspace>,
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
        let request = Request {
            path: &file.file_path,
            lines: file.lines,
            numbered: file.numbered.unwrap_or(true),
            fold_level: file.fold_level,
            symbols: file.symbol.as_ref().map(Names::cleaned),
            at,
        };
        match reader::read(workspace, &request) {
            Ok(read) => {
                blocks.push(read.text);
                unfocused = unfocused.or(read.unfocused);
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
