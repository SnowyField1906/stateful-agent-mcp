//! Parameters for language tooling tools.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};

use super::common::{PageParams, flex_u64};

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrManyKinds {
    One(String),
    Many(Vec<String>),
}

impl From<OneOrManyKinds> for Vec<String> {
    fn from(v: OneOrManyKinds) -> Self {
        match v {
            OneOrManyKinds::One(s) => vec![s],
            OneOrManyKinds::Many(v) => v,
        }
    }
}

fn opt_kinds_map<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<BTreeMap<String, Vec<String>>>, D::Error> {
    Ok(Option::<BTreeMap<String, OneOrManyKinds>>::deserialize(d)?
        .map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect()))
}

/// Definitions (functions, types, modules, ...) in a file or below a directory.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CodeSymbolsParams {
    /// File or directory, relative to the project root or any absolute path on disk. Default `.`.
    #[serde(default)]
    pub implicit_path: Option<String>,
    /// Only symbols whose name contains this text (case-insensitive).
    #[serde(default)]
    pub query: Option<String>,
    /// Symbols to include per language (`{ [language]: [kinds...] }`), for example
    /// `{"rust": ["fn", "struct", "enum", "trait", "impl", "mod", "const", "type"], "python": ["def", "class", "var"]}`.
    #[serde(default, deserialize_with = "opt_kinds_map")]
    #[schemars(with = "Option<BTreeMap<String, Vec<String>>>")]
    pub kinds: Option<BTreeMap<String, Vec<String>>>,
    /// Include dotfiles and gitignored files.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

/// Where a name is defined and used in Rust or Python code, ignoring comments and strings.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CodeReferencesParams {
    /// Name to look up: a function, type, variable, field or module name.
    pub symbol: String,
    /// File or directory, relative to the project root or any absolute path on disk. Default `.`.
    #[serde(default)]
    pub implicit_path: Option<String>,
    /// Include dotfiles and gitignored files.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

/// What a function or method calls and what calls it, in Rust or Python code.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CodeCallsParams {
    /// Name of the function or method.
    pub symbol: String,
    /// File or directory to index, relative to the project root or any absolute path on disk.
    /// Default `.`. Callers outside it are not found.
    #[serde(default)]
    pub implicit_path: Option<String>,
    /// Only the definition in this file, as shown in the output, when several share the name.
    #[serde(default)]
    pub file_path: Option<String>,
    /// Only the definition that contains this 1-based line.
    #[serde(default, deserialize_with = "flex_u64")]
    pub line: Option<u64>,
    /// `incoming` (callers), `outgoing` (callees) or `both` (default).
    #[serde(default)]
    pub direction: Option<String>,
    /// Call tree levels to follow (default 1, max 3).
    #[serde(default, deserialize_with = "flex_u64")]
    pub tree_depth: Option<u64>,
    /// Include dotfiles and gitignored files.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
}

/// Run a command configured in `.AGENTS/config.toml`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunProjectActionParams {
    /// Language section of the configuration, for example `rust` or `python`.
    pub language: String,
    /// One of `lint`, `build`, `test`, `install`, `remove`.
    pub action: String,
    /// Arguments, for commands configured with `{args}`. Pass an empty string to run such a
    /// command without arguments, or `--help` to see what it accepts.
    #[serde(default)]
    pub args: Option<String>,
}
