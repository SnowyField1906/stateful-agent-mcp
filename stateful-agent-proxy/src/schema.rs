//! Tool schema inspection, relaxation, and compact diffing between worker generations.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

/// Snapshot of one MCP tool definition from `tools/list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub raw: Value,
}

impl ToolInfo {
    pub fn from_value(raw: Value) -> Option<Self> {
        let name = raw.get("name")?.as_str()?.to_string();
        let description = raw
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let input_schema = raw
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object"}));
        Some(Self {
            name,
            description,
            input_schema,
            raw,
        })
    }

    /// Compact parameter signature: `tool_name(req_param: type, opt_param?: type)`.
    pub fn signature(&self) -> String {
        let params = extract_params(&self.input_schema);
        let list = params
            .into_iter()
            .map(|(name, ty, req)| {
                if req {
                    format!("{name}: {ty}")
                } else {
                    format!("{name}?: {ty}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}({list})", self.name)
    }

    pub fn short_description(&self) -> String {
        let first = self
            .description
            .lines()
            .next()
            .unwrap_or("")
            .split(". ")
            .next()
            .unwrap_or("")
            .trim();
        if first.chars().count() <= 100 {
            first.to_string()
        } else {
            let cut: String = first.chars().take(100).collect();
            format!("{cut}…")
        }
    }
}

pub fn parse_tools(result: &Value) -> Vec<ToolInfo> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .cloned()
                .filter_map(ToolInfo::from_value)
                .collect()
        })
        .unwrap_or_default()
}

/// Remove `"additionalProperties": false` from initial tool schemas so clients validating
/// locally do not block newly added optional fields on existing tools.
pub fn relax_tool_schema(mut raw: Value) -> Value {
    if let Some(schema) = raw.get_mut("inputSchema").and_then(Value::as_object_mut)
        && schema.get("additionalProperties") == Some(&Value::Bool(false))
    {
        schema.remove("additionalProperties");
    }
    raw
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SchemaDiff {
    pub added: Vec<ToolInfo>,
    pub modified: Vec<ToolChange>,
    pub removed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolChange {
    pub name: String,
    pub signature: String,
    pub added_params: Vec<String>,
    pub removed_params: Vec<String>,
    pub changed_params: Vec<String>,
    pub description_changed: bool,
}

impl SchemaDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.modified.is_empty() && self.removed.is_empty()
    }

    pub fn render_compact(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut lines = Vec::new();
        for tool in &self.added {
            lines.push(format!(
                "  + {} — {}",
                tool.signature(),
                tool.short_description()
            ));
        }
        for m in &self.modified {
            let mut details = Vec::new();
            if !m.added_params.is_empty() {
                details.push(format!("+params: {}", m.added_params.join(", ")));
            }
            if !m.removed_params.is_empty() {
                details.push(format!("-params: {}", m.removed_params.join(", ")));
            }
            if !m.changed_params.is_empty() {
                details.push(format!("~params: {}", m.changed_params.join(", ")));
            }
            if details.is_empty() && m.description_changed {
                details.push("description updated".to_string());
            }
            lines.push(format!("  ~ {} ({})", m.signature, details.join("; ")));
        }
        for name in &self.removed {
            lines.push(format!("  - {name} (removed)"));
        }
        Some(lines.join("\n"))
    }
}

pub fn diff_tools(old: &[ToolInfo], new: &[ToolInfo]) -> SchemaDiff {
    let old_map: BTreeMap<&str, &ToolInfo> = old.iter().map(|t| (t.name.as_str(), t)).collect();
    let new_map: BTreeMap<&str, &ToolInfo> = new.iter().map(|t| (t.name.as_str(), t)).collect();

    let mut diff = SchemaDiff::default();
    for tool in new {
        match old_map.get(tool.name.as_str()) {
            None => diff.added.push(tool.clone()),
            Some(prev) => {
                if let Some(change) = diff_one(prev, tool) {
                    diff.modified.push(change);
                }
            }
        }
    }
    for prev in old {
        if !new_map.contains_key(prev.name.as_str()) {
            diff.removed.push(prev.name.clone());
        }
    }
    diff
}

fn diff_one(old: &ToolInfo, new: &ToolInfo) -> Option<ToolChange> {
    if old.input_schema == new.input_schema && old.description == new.description {
        return None;
    }
    let old_params: BTreeMap<String, (String, bool)> = extract_params(&old.input_schema)
        .into_iter()
        .map(|(n, ty, req)| (n, (ty, req)))
        .collect();
    let new_params: BTreeMap<String, (String, bool)> = extract_params(&new.input_schema)
        .into_iter()
        .map(|(n, ty, req)| (n, (ty, req)))
        .collect();

    let mut added_params = Vec::new();
    let mut removed_params = Vec::new();
    let mut changed_params = Vec::new();

    for (name, (ty, req)) in &new_params {
        match old_params.get(name) {
            None => added_params.push(if *req {
                format!("{name}: {ty}")
            } else {
                format!("{name}?: {ty}")
            }),
            Some(prev) if prev != &(ty.clone(), *req) => changed_params.push(if *req {
                format!("{name}: {ty}")
            } else {
                format!("{name}?: {ty}")
            }),
            Some(_) => {}
        }
    }
    for name in old_params.keys() {
        if !new_params.contains_key(name) {
            removed_params.push(name.clone());
        }
    }
    let description_changed = old.description != new.description;
    if added_params.is_empty()
        && removed_params.is_empty()
        && changed_params.is_empty()
        && !description_changed
        && old.input_schema != new.input_schema
    {
        changed_params.push("schema".to_string());
    }
    Some(ToolChange {
        name: new.name.clone(),
        signature: new.signature(),
        added_params,
        removed_params,
        changed_params,
        description_changed,
    })
}

fn extract_params(schema: &Value) -> Vec<(String, String, bool)> {
    let required: BTreeSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String, bool)> = props
        .iter()
        .map(|(name, prop)| {
            let req = required.contains(name.as_str());
            (name.clone(), type_label(prop), req)
        })
        .collect();
    out.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    out
}

fn type_label(prop: &Value) -> String {
    if let Some(ty) = prop.get("type") {
        match ty {
            Value::String(s) => return s.clone(),
            Value::Array(arr) => {
                let parts: Vec<&str> = arr
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|&t| t != "null")
                    .collect();
                if !parts.is_empty() {
                    return parts.join("|");
                }
            }
            _ => {}
        }
    }
    if prop.get("anyOf").is_some() || prop.get("oneOf").is_some() {
        return "any".to_string();
    }
    "value".to_string()
}
