//! Finding text in the recorded turns: a regular expression over titles, notes and handover
//! tasks, newest turn first. `workflow_search` pages the hits.

use std::collections::BTreeSet;

use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};

use super::Project;
use super::session::model::TurnRecord;
use crate::common::{AgentError, Result};

/// Longest text shown for a search hit.
const SNIPPET_CHARS: usize = 160;
const KINDS: [&str; 3] = ["title", "note", "task"];

/// What to look for and where.
pub struct Query<'a> {
    /// A regular expression, or plain text when `literal`.
    pub text: &'a str,
    pub literal: bool,
    pub case_insensitive: bool,
    /// Any of `title`, `note`, `task`, also in the plural; all three when `None` or empty.
    pub kinds: Option<&'a [String]>,
    /// Sessions, turns or ranges to search; all turns when `None`.
    pub within: Option<&'a [String]>,
}

/// The hits `{id, kind, text}` in the turns of `query.within`, newest turn first.
pub fn hits(project: &Project, query: &Query<'_>) -> Result<Vec<Value>> {
    if query.text.is_empty() {
        return Err(AgentError::invalid("query", "must not be empty"));
    }
    let pattern = if query.literal {
        regex::escape(query.text)
    } else {
        query.text.to_string()
    };
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(query.case_insensitive)
        .build()
        .map_err(|e| AgentError::invalid("query", e.to_string()))?;
    let kinds = search_kinds(query.kinds)?;
    let flat = project.flat_keys();
    let indexes = project.turn_indexes(&flat, query.within)?;

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
    Ok(hits)
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
