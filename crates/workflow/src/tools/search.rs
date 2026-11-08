//! `workflow_search`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Project;
use crate::common::params::{PageParams, opt_string_or_vec};
use crate::common::{Result, ServerConfig};
use crate::engine::lookup::{self, Query};

pub const DESCRIPTION: &str = "Search the titles, notes and handover tasks of recorded turns, newest first, by regex (or plain text with `literal`). Each hit is `{id, kind, text}`; pass the ID to the `workflow_get_*` tools. Use it to find earlier decisions before repeating work.";

/// Parameters for `workflow_search`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// Regular expression (or literal text with `literal`).
    pub query: String,
    /// Sessions, turns or ranges to search (default: all turns).
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    pub implicit_ids: Option<Vec<String>>,
    /// What to search: any of `title`, `note`, `task` (default: all three).
    #[serde(default, deserialize_with = "opt_string_or_vec")]
    pub kinds: Option<Vec<String>>,
    /// Treat `query` as literal text.
    #[serde(default)]
    pub literal: Option<bool>,
    #[serde(default)]
    pub case_insensitive: Option<bool>,
    #[serde(flatten)]
    pub page: PageParams,
}

/// Search titles, notes and handover tasks, newest turn first.
pub fn search(project: &Project, cfg: &ServerConfig, p: SearchParams) -> Result<Value> {
    let query = Query {
        text: &p.query,
        literal: p.literal.unwrap_or(false),
        case_insensitive: p.case_insensitive.unwrap_or(false),
        kinds: p.kinds.as_deref(),
        within: p.implicit_ids.as_deref(),
    };
    let hits = lookup::hits(project, &query)?;

    let total = hits.len();
    let (offset, len) = p.page.window_in(cfg.default_limit, total)?;
    let page: Vec<Value> = hits.into_iter().skip(offset).take(len).collect();
    let shown = page.len();
    let mut out = json!({ "hits": page, "total": total });
    if shown > 0 && shown < total {
        out["showing"] = json!(format!("{}..{}", offset + 1, offset + shown));
    }
    Ok(out)
}
