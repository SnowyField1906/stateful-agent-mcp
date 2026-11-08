//! `filesystem_find`: recursive search by glob, kind and size.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::common::fs::resolve::Scope;
use crate::common::fs::walk::{Kind, SortBy};
use crate::common::params::{PageParams, opt_string_or_vec};
use crate::common::range::{Range, opt_range};
use crate::common::{Result, ServerConfig, Workspace, root_of};
use crate::filesystem::engine::listing::{ListSpec, list};

pub const DESCRIPTION: &str = "Find files by `glob` (gitignore syntax: `*.rs` matches at any depth, `src/**/*.ts` is anchored), `kind`, `size` and `sort_by`. Respects `.gitignore`.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FindParams {
    /// Directory to search, relative to the project root or absolute. Default `.`.
    #[serde(default)]
    pub folder_path: Option<String>,
    /// Glob(s) matched like `.gitignore` patterns: `*.rs` matches at any depth,
    /// `src/**/*.ts` is anchored. Omit to match everything.
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    pub glob: Option<Vec<String>>,
    /// `file` (default), `dir` or `symlink`.
    #[serde(default)]
    pub kind: Option<Kind>,
    /// File size in bytes, for example `100..1024` or `1024..`.
    #[serde(default, deserialize_with = "opt_range")]
    #[schemars(with = "Option<String>")]
    pub size: Option<Range>,
    /// Include dotfiles and gitignored entries.
    #[serde(default)]
    pub show_gitignored: Option<bool>,
    /// `name` (default), `size` (largest first), `modified` (newest first) or `extension`.
    #[serde(default)]
    pub sort_by: Option<SortBy>,
    #[serde(default)]
    pub reverse: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

pub fn find(
    workspace: Option<&dyn Workspace>,
    cfg: &ServerConfig,
    p: FindParams,
) -> Result<String> {
    let scope = Scope::new(root_of(workspace), p.folder_path.as_deref(), "folder_path")?;
    let kind = p.kind.unwrap_or(Kind::File);
    list(
        &scope,
        ListSpec {
            depth: cfg.max_dir_depth.max(1),
            show_gitignored: p.show_gitignored.unwrap_or(false),
            globs: p.glob.unwrap_or_default(),
            kind: Some(kind),
            size: p.size,
            sort_by: p.sort_by,
            reverse: p.reverse.unwrap_or(false),
            long: false,
            dirs_first: false,
            relative_names: false,
            page: p.page,
            default_limit: cfg.default_limit,
        },
    )
}
