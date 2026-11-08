//! `langtool_find_symbols`: definitions in a file or below a directory, for languages with a
//! [`Syntax`](crate::language::Syntax). Output, grouped by file:
//! ```text
//! src/a.rs
//! 5..9\timpl P
//! 6..8\t  fn new() -> Self
//! ```

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};

use crate::common::params::{Names, PageParams};
use crate::common::{Result, ServerConfig, Workspace};
use crate::engine::definitions::{self, Listing};
use crate::engine::scan::ScanTarget;
use crate::engine::scope::scope_filter;

pub const DESCRIPTION: &str = "List definitions in files with a registered Tree-sitter grammar, in one file or below a directory. Each row is the source lines and the signature, indented under its parent definition, or ending with `(in Parent::Path)` when the parent is not listed; filter by `query` (one name or a list, each a name substring; `Type::name` also requires `Type` to enclose it), `scope` (enclosing-scope substring) or `kinds` (language-to-kind map; errors list allowed kinds). Language-specific nested items are opt-in; `docs` adds the first documentation line; `include_body` prints the source below the row of each listed definition of up to 50 lines that has no listed definition inside it, as rows `  N<tab>text`. Use the ranges with `langtool_read_file`.";

/// Definitions (functions, types, modules, ...) in a file or below a directory.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FindSymbolsParams {
    /// File or directory, relative to the project root or absolute. Default `.`.
    #[serde(default)]
    pub implicit_path: Option<String>,
    /// Only symbols whose name contains this text (case-insensitive); a list shows the symbols
    /// that match any of its entries. A qualified query such as `Store::put`, `Store.put` or
    /// `Store/put` also requires `Store` to enclose the symbol.
    #[serde(default)]
    #[schemars(with = "Option<Vec<String>>")]
    pub query: Option<Names>,
    /// Only symbols whose enclosing scope, written as a qualified path (`east::Token`), contains
    /// this text (case-insensitive).
    #[serde(default)]
    pub scope: Option<String>,
    /// Kinds to include by language (`{ [language]: [kinds...] }`), for example `{"rust": ["fn", "struct"]}`. Valid kinds depend on the registered grammar.
    #[serde(default, deserialize_with = "opt_kinds_map")]
    #[schemars(with = "Option<BTreeMap<String, Vec<String>>>")]
    pub kinds: Option<BTreeMap<String, Vec<String>>>,
    /// Include dotfiles and gitignored files.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
    /// Add the first documentation line of each definition as a fourth column.
    #[serde(default)]
    pub docs: Option<bool>,
    /// Print the source of each listed definition of up to 50 lines below its row (rows of
    /// `  N<tab>text`), unless another listed definition lies inside it. Default false.
    #[serde(default)]
    pub include_body: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

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

pub fn find_symbols(
    workspace: Option<&dyn Workspace>,
    cfg: &ServerConfig,
    p: FindSymbolsParams,
) -> Result<String> {
    let queries = p.query.as_ref().map(Names::cleaned).unwrap_or_default();
    let query = Listing {
        queries: &queries,
        scope: scope_filter(p.scope.as_deref()),
        kinds: p.kinds.as_ref(),
        target: ScanTarget {
            path: p.implicit_path.as_deref(),
            max_depth: cfg.max_dir_depth,
            show_gitignored: p.show_gitignored,
        },
        docs: p.docs.unwrap_or(false),
        include_body: p.include_body.unwrap_or(false),
        page: &p.page,
    };
    definitions::list(workspace, cfg, &query)
}
