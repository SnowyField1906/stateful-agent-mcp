//! `workflow_search`.

use std::collections::BTreeSet;

use regex::{Regex, RegexBuilder};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::common::params::{PageParams, opt_string_or_vec};
use crate::common::{AgentError, Result, ServerConfig};
use crate::workflow::Project;
use crate::workflow::engine::session::model::TurnRecord;

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

/// Longest text shown for a search hit.
const SNIPPET_CHARS: usize = 160;
const KINDS: [&str; 3] = ["title", "note", "task"];

/// Search titles, notes and handover tasks, newest turn first.
pub fn search(project: &Project, cfg: &ServerConfig, p: SearchParams) -> Result<Value> {
    if p.query.is_empty() {
        return Err(AgentError::invalid("query", "must not be empty"));
    }
    let pattern = if p.literal.unwrap_or(false) {
        regex::escape(&p.query)
    } else {
        p.query.clone()
    };
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(p.case_insensitive.unwrap_or(false))
        .build()
        .map_err(|e| AgentError::invalid("query", e.to_string()))?;
    let kinds = search_kinds(p.kinds.as_deref())?;
    let flat = project.flat_keys();
    let indexes = project.turn_indexes(&flat, p.implicit_ids.as_deref())?;

    let mut hits: Vec<Value> = Vec::new();
    for i in indexes.iter().rev() {
        let Some(turn) = project.turn(flat[*i]) else {
            continue;
        };
        if kinds.contains("title")
            && let Some(title) = &turn.title
            && re.is_match(title)
        {
            hits.push(json!({
                "id": turn.key.to_string(),
                "kind": "title",
                "text": clip(title),
            }));
        }
        if kinds.contains("note") {
            for (idx, note) in turn.notes.iter().enumerate() {
                if re.is_match(&note.content) {
                    hits.push(json!({
                        "id": TurnRecord::note_id(turn.key, idx),
                        "kind": "note",
                        "text": snippet(&re, &note.content),
                    }));
                }
            }
        }
        if kinds.contains("task") {
            for task in turn.task_dtos() {
                if re.is_match(&task.content) {
                    let mut hit = json!({
                        "id": task.id,
                        "kind": "task",
                        "text": snippet(&re, &task.content),
                    });
                    if task.user_explicit {
                        hit["user_explicit"] = json!(true);
                    }
                    hits.push(hit);
                }
            }
        }
    }

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

/// Kinds to search; singular and plural spellings are accepted.
fn search_kinds(raw: Option<&[String]>) -> Result<BTreeSet<&'static str>> {
    let raw = raw.unwrap_or_default();
    if raw.is_empty() {
        return Ok(KINDS.into_iter().collect());
    }
    raw.iter()
        .map(|k| {
            let lower = k.trim().to_ascii_lowercase();
            let key = lower.strip_suffix('s').unwrap_or(&lower);
            KINDS.into_iter().find(|c| *c == key).ok_or_else(|| {
                AgentError::invalid(
                    "kinds",
                    format!("`{}` is not title, note or task", k.trim()),
                )
            })
        })
        .collect()
}

/// The first line of `text` that matches, clipped.
fn snippet(re: &Regex, text: &str) -> String {
    let line = text
        .lines()
        .find(|l| re.is_match(l))
        .or_else(|| text.lines().next())
        .unwrap_or("");
    clip(line)
}

fn clip(line: &str) -> String {
    let line = line.trim();
    if line.chars().count() <= SNIPPET_CHARS {
        return line.to_string();
    }
    let cut: String = line.chars().take(SNIPPET_CHARS).collect();
    format!("{cut}…")
}
