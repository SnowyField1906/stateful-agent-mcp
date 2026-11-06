//! Gateway tools injected into `tools/list` so newly added or schema-updated tools can be
//! called without restarting or refreshing the MCP client.

use serde_json::{Value, json};

use crate::schema::{SchemaDiff, ToolInfo};

pub const CALL_TOOL: &str = "proxy_call_tool";
pub const LIST_TOOLS: &str = "proxy_list_tools";
pub const RELOAD_TOOL: &str = "proxy_reload";

pub fn is_gateway_tool(name: &str) -> bool {
    matches!(name, CALL_TOOL | LIST_TOOLS | RELOAD_TOOL)
}

pub fn gateway_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": CALL_TOOL,
            "description": "Call any tool on the live MCP worker by name, including tools added or schema-updated after the client session started.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tool": {
                        "type": "string",
                        "description": "Exact name of the worker tool to invoke."
                    },
                    "arguments": {
                        "type": "object",
                        "description": "Arguments object to pass to the target tool."
                    }
                },
                "required": ["tool"]
            }
        }),
        json!({
            "name": LIST_TOOLS,
            "description": "List tools currently live on the MCP worker (reloads first if the binary changed on disk). Use `diff_only: true` to see only tools added or modified since session startup, or `full_schema: true` for raw JSON schemas.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Optional case-insensitive substring filter on tool name."
                    },
                    "diff_only": {
                        "type": "boolean",
                        "description": "Show only tools added or modified since the client session started."
                    },
                    "full_schema": {
                        "type": "boolean",
                        "description": "Include the full JSON inputSchema for each matching tool."
                    }
                }
            }
        }),
        json!({
            "name": RELOAD_TOOL,
            "description": "Check if the MCP server binary on disk was rebuilt (or force a restart with `force: true`), hot-reload the worker process, replay project registration, and report any tool schema changes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "force": {
                        "type": "boolean",
                        "description": "Restart the worker process even if the binary hash on disk has not changed."
                    }
                }
            }
        }),
    ]
}

pub fn unwrap_call_tool(args: Option<&Value>) -> Result<(String, Value), String> {
    let Some(obj) = args.and_then(Value::as_object) else {
        return Err("`proxy_call_tool` requires an arguments object with `tool`".into());
    };
    let tool = ["tool", "name", "tool_name"]
        .into_iter()
        .find_map(|k| obj.get(k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "missing `tool` (name of the worker tool to call)".to_string())?;

    if is_gateway_tool(tool) {
        return Err(format!("`{tool}` is a proxy gateway tool; call it directly"));
    }

    let raw_inner = ["arguments", "args", "params"]
        .into_iter()
        .find_map(|k| obj.get(k));
    let inner = match raw_inner {
        None | Some(Value::Null) => json!({}),
        Some(Value::Object(map)) => Value::Object(map.clone()),
        Some(Value::String(s)) if s.trim().is_empty() => json!({}),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s)
            .map_err(|e| format!("`arguments` string is not valid JSON: {e}"))?,
        Some(other) => {
            return Err(format!("`arguments` must be a JSON object, got {other}"));
        }
    };
    Ok((tool.to_string(), inner))
}

pub fn render_tool_list(
    live_tools: &[ToolInfo],
    diff_vs_client: &SchemaDiff,
    args: Option<&Value>,
) -> String {
    let query = args
        .and_then(|a| a.get("query"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    let diff_only = args
        .and_then(|a| a.get("diff_only"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let full_schema = args
        .and_then(|a| a.get("full_schema"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if diff_only && query.is_none() && !full_schema {
        return match diff_vs_client.render_compact() {
            Some(text) => format!("Tool schema changes since session startup:\n{text}\n"),
            None => "No tool schema changes since session startup.\n".to_string(),
        };
    }

    let changed_names = |name: &str| -> bool {
        diff_vs_client.added.iter().any(|t| t.name == name)
            || diff_vs_client.modified.iter().any(|m| m.name == name)
    };

    let matched: Vec<&ToolInfo> = live_tools
        .iter()
        .filter(|t| !diff_only || changed_names(&t.name))
        .filter(|t| {
            query
                .as_ref()
                .is_none_or(|q| t.name.to_lowercase().contains(q))
        })
        .collect();

    if matched.is_empty() {
        return "No matching tools.\n".to_string();
    }

    let mut out = String::new();
    for tool in matched {
        let marker = if diff_vs_client.added.iter().any(|t| t.name == tool.name) {
            "+ "
        } else if diff_vs_client.modified.iter().any(|m| m.name == tool.name) {
            "~ "
        } else {
            "  "
        };
        out.push_str(&format!(
            "{marker}{} — {}\n",
            tool.signature(),
            tool.short_description()
        ));
        if full_schema {
            out.push_str(&format!("    schema: {}\n", tool.input_schema));
        }
    }
    out
}