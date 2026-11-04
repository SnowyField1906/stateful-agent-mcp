//! Parameters of the read-only filesystem tools.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::common::{PageParams, flex_u64, opt_string_or_vec};
use super::range::{Range, opt_range};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FsLsParams {
    /// Directory path, relative to the project root or any absolute path on disk. Default `.`.
    #[serde(default)]
    pub folder_path: Option<String>,
    /// List subdirectories too. Respects `.gitignore` unless `show_gitignored` is set.
    #[serde(default)]
    pub recursive: Option<bool>,
    /// Include dotfiles and gitignored entries.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
    /// Glob filter(s) such as `*.rs` or `src/**/*.ts`.
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    pub glob: Option<Vec<String>>,
    /// File size in bytes, for example `100..1024` or `1024..`.
    #[serde(default, deserialize_with = "opt_range")]
    #[schemars(with = "Option<String>")]
    pub size: Option<Range>,
    /// `file`, `dir` or `symlink`.
    #[serde(default)]
    pub kind: Option<String>,
    /// `name` (default), `size` (largest first), `modified` (newest first) or `extension`.
    #[serde(default)]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub reverse: Option<bool>,
    /// Add permission bits and modification time (unix seconds).
    #[serde(default)]
    pub long: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FsFindParams {
    /// Directory to search, relative to the project root or any absolute path on disk. Default `.`.
    #[serde(default)]
    pub folder_path: Option<String>,
    /// Glob(s) matched like `.gitignore` patterns: `*.rs` matches at any depth,
    /// `src/**/*.ts` is anchored. Omit to match everything.
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    pub glob: Option<Vec<String>>,
    /// `file` (default), `dir` or `symlink`.
    #[serde(default)]
    pub kind: Option<String>,
    /// File size in bytes, for example `100..1024` or `1024..`.
    #[serde(default, deserialize_with = "opt_range")]
    #[schemars(with = "Option<String>")]
    pub size: Option<Range>,
    /// Include dotfiles and gitignored entries.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
    /// `name` (default), `size` (largest first), `modified` (newest first) or `extension`.
    #[serde(default)]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub reverse: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FsReadFilesParams {
    /// Files to read, each with its own `file_path`, optional `lines`, `numbered` and `fold_level`.
    #[serde(deserialize_with = "files_list")]
    #[schemars(with = "Vec<FsReadFileParams>")]
    pub files: Vec<FsReadFileParams>,
    /// Read files as they were at the end of this turn (`01.02`) instead of the current state.
    #[serde(default)]
    pub at_implicit_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FsReadFileParams {
    /// File path, relative to the project root or any absolute path on disk.
    pub file_path: String,
    /// Lines to read, 1-based and inclusive: `10..50`, `10..` (400 lines), `..50` or `-30..`
    /// (last 30). At most 2000 lines. Default `1..400`.
    #[serde(default, deserialize_with = "opt_range")]
    #[schemars(with = "Option<String>")]
    pub lines: Option<Range>,
    /// Prefix lines with their number (default true).
    #[serde(default)]
    pub numbered: Option<bool>,
    /// Fold AST blocks nested deeper than this level into `A..B` rows (Rust and Python only).
    /// `0` shows only top-level declarations, `1` also their bodies' first level; at most 16.
    /// Omit to read raw lines without folding.
    #[serde(default, deserialize_with = "flex_u64")]
    pub fold_level: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FsGrepParams {
    /// Regular expression (or literal text with `literal`).
    pub query: String,
    /// File or directory, relative to the project root or any absolute path on disk. Default `.`.
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
    pub output: Option<String>,
    /// Include dotfiles and gitignored files.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FsFileInfoParams {
    /// File or directory, relative to the project root or any absolute path on disk.
    #[serde(default)]
    pub implicit_path: Option<String>,
}

/// `files` as a list of file requests. Also accepts a JSON string holding that list, a single
/// request object, or plain path strings; anything else gets a hint about the expected shape.
fn files_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<FsReadFileParams>, D::Error> {
    use serde::de::Error;
    const HINT: &str = r#"`files` must be an array of objects, for example [{"file_path": "src/a.rs", "lines": "1..80"}]"#;

    let value = match Value::deserialize(d)? {
        Value::String(raw) => serde_json::from_str::<Value>(&raw).map_err(|_| <D::Error as Error>::custom(HINT))?,
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
            Value::String(file_path) => Ok(FsReadFileParams {
                file_path,
                lines: None,
                numbered: None,
                fold_level: None,
            }),
            other => serde_json::from_value(other).map_err(|e| Error::custom(format!("{HINT}: {e}"))),
        })
        .collect()
}
