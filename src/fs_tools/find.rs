//! `filesystem_find`: recursive search by glob, kind and size.

use crate::config::ServerConfig;
use crate::error::Result;
use crate::params::FsFindParams;
use crate::state::Project;

use super::ls::{ListSpec, list};
use super::resolve::Scope;
use super::walk::Kind;

pub fn find(project: Option<&Project>, cfg: &ServerConfig, p: FsFindParams) -> Result<String> {
    let scope = Scope::new(project, p.folder_path.as_deref())?;
    let kind = match p.kind.as_deref() {
        Some(raw) => Kind::parse(raw)?,
        None => Kind::File,
    };
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
