//! `filesystem_ls`: list a directory.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::common::fs::resolve::Scope;
use crate::common::fs::walk::{Kind, SortBy};
use crate::common::params::{PageParams, opt_string_or_vec};
use crate::common::range::{Range, opt_range};
use crate::common::{Result, ServerConfig, Workspace, root_of};
use crate::filesystem::engine::listing::{ListSpec, list};

pub const DESCRIPTION: &str = "List a directory with sizes, directories first. Respects `.gitignore`; dotfiles are hidden unless `show_gitignored`.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct LsParams {
    /// Directory, relative to the project root or absolute. Default `.`.
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
    pub kind: Option<Kind>,
    /// `name` (default), `size` (largest first), `modified` (newest first) or `extension`.
    #[serde(default)]
    pub sort_by: Option<SortBy>,
    #[serde(default)]
    pub reverse: Option<bool>,
    /// Add permission bits and modification time (unix seconds).
    #[serde(default)]
    pub long: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

pub fn ls(workspace: Option<&dyn Workspace>, cfg: &ServerConfig, p: LsParams) -> Result<String> {
    let scope = Scope::new(root_of(workspace), p.folder_path.as_deref(), "folder_path")?;
    let recursive = p.recursive.unwrap_or(false);
    let spec = ListSpec {
        depth: if recursive {
            cfg.max_dir_depth.max(1)
        } else {
            1
        },
        show_gitignored: p.show_gitignored.unwrap_or(false),
        globs: p.glob.unwrap_or_default(),
        kind: p.kind,
        size: p.size,
        sort_by: p.sort_by,
        reverse: p.reverse.unwrap_or(false),
        long: p.long.unwrap_or(false),
        dirs_first: !recursive,
        relative_names: true,
        page: p.page,
        default_limit: cfg.default_limit,
    };
    list(&scope, spec)
}
