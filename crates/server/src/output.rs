//! Conversion of results into MCP tool results.
//!
//! Results are sent once, as compact text. Structured content is not duplicated
//! to avoid doubling the token cost.

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;

use crate::common::{AgentError, Result};

/// Error result (`isError: true`).
pub fn error_result(err: &AgentError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(err.to_string())])
}

/// JSON result. Fails (instead of truncating) when over `max_bytes`, because cut JSON is unusable.
pub fn json_result(res: Result<Value>, max_bytes: usize) -> CallToolResult {
    match res {
        Ok(value) => {
            let text = value.to_string();
            if text.len() > max_bytes {
                return error_result(&AgentError::invalid(
                    "result",
                    format!(
                        "output is {} bytes, over the {max_bytes}-byte limit; narrow the request (for example with `implicit_paths` or a single turn selector)",
                        text.len()
                    ),
                ));
            }
            CallToolResult::success(vec![ContentBlock::text(text)])
        }
        Err(err) => error_result(&err),
    }
}

/// Plain-text result, cut on a line boundary when over `max_bytes`.
pub fn text_result(res: Result<String>, max_bytes: usize) -> CallToolResult {
    match res {
        Ok(text) => {
            CallToolResult::success(vec![ContentBlock::text(truncate_text(text, max_bytes))])
        }
        Err(err) => error_result(&err),
    }
}

/// Cut `text` to at most `max_bytes`, preferring a line boundary, and append a notice.
pub fn truncate_text(text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if let Some(nl) = text[..end].rfind('\n') {
        end = nl + 1;
    }
    let total = text.len();
    let mut cut = text;
    cut.truncate(end);
    if !cut.ends_with('\n') {
        cut.push('\n');
    }
    cut.push_str(&format!(
        "[truncated: {end} of {total} bytes; narrow the request]\n"
    ));
    cut
}
