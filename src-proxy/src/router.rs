//! Stateful MCP proxy router: manages worker hot-swapping, automatic `workflow_register` replay,
//! turn-based build version tracking, and gateway tool dispatch.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::binary::{BinaryManager, BuildVersionStatus};
use crate::gateway::{
    CALL_TOOL, LIST_TOOLS, RELOAD_TOOL, gateway_tool_definitions, render_tool_list,
    unwrap_call_tool,
};
use crate::schema::{SchemaDiff, ToolInfo, diff_tools, parse_tools, relax_tool_schema};
use crate::worker::Worker;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(900);
const REGISTER_TOOL: &str = "workflow_register";

const PROXY_INSTRUCTION_SUFFIX: &str = "Hot-reload proxy active: when the server binary is rebuilt, it reloads automatically and replays `workflow_register`. If tools or parameter schemas change mid-session, call them via `proxy_call_tool` (or inspect with `proxy_list_tools`) without restarting the client.";

pub struct ProxyRouter {
    worker_args: Vec<String>,
    binaries: BinaryManager,
    worker: Option<Worker>,
    next_generation: u64,
    init_params: Option<Value>,
    client_baseline_tools: Vec<ToolInfo>,
    /// Cached `workflow_register` arguments for automatic replay on worker reload.
    register_args: Option<Value>,
    /// Active workspace root (bootstrapped at startup or set by `workflow_register`).
    workspace_root: Option<PathBuf>,
    pending_notice: Option<String>,
    client_tx: mpsc::UnboundedSender<String>,
}

pub struct ReloadReport {
    pub prev_build_turn: Option<String>,
    pub new_build_turn: String,
    pub active_turn_id: Option<String>,
    pub diff_vs_prev: SchemaDiff,
    pub diff_vs_client: SchemaDiff,
}

impl ReloadReport {
    pub fn format_notice(&self) -> String {
        let turn_transition = match &self.prev_build_turn {
            Some(prev) if prev != &self.new_build_turn => {
                format!("build turn {prev} -> {}", self.new_build_turn)
            }
            _ => format!("build turn {}", self.new_build_turn),
        };
        let active_info = self
            .active_turn_id
            .as_deref()
            .map(|id| format!("; active_turn_id={id}"))
            .unwrap_or_default();
        let mut out = format!("[proxy: hot-reloaded worker ({turn_transition}{active_info})]");
        if let Some(diff) = self.diff_vs_prev.render_compact() {
            out.push_str(&format!(
                "\n[proxy: tool changes in this build:\n{diff}\nUse `{CALL_TOOL}` to invoke new or schema-updated tools without restarting the client.]"
            ));
        } else if let Some(diff) = self.diff_vs_client.render_compact() {
            out.push_str(&format!(
                "\n[proxy: tool changes since session start:\n{diff}\nUse `{CALL_TOOL}` to invoke new or schema-updated tools.]"
            ));
        }
        out
    }
}

impl ProxyRouter {
    pub fn new(
        primary_bin: PathBuf,
        worker_args: Vec<String>,
        bootstrap_workspace: Option<PathBuf>,
        client_tx: mpsc::UnboundedSender<String>,
    ) -> anyhow::Result<Self> {
        let binaries = BinaryManager::new(primary_bin)?;
        Ok(Self {
            worker_args,
            binaries,
            worker: None,
            next_generation: 1,
            init_params: None,
            client_baseline_tools: Vec::new(),
            register_args: None,
            workspace_root: bootstrap_workspace,
            pending_notice: None,
            client_tx,
        })
    }

    fn current_version_status(&self) -> Option<BuildVersionStatus> {
        let worker = self.worker.as_ref()?;
        Some(
            self.binaries
                .resolve_version_status(&worker.fingerprint, self.workspace_root.as_deref()),
        )
    }

    pub async fn poll_background_reload(&mut self) {
        if self.init_params.is_none() {
            return;
        }
        let current_fp = self.worker.as_ref().map(|w| &w.fingerprint);
        if !self.binaries.has_changed_meta(current_fp) {
            return;
        }
        if let Ok(Some(report)) = self.reload_if_needed(false).await {
            self.push_pending_notice(report.format_notice());
        }
    }

    pub async fn handle_client_line(&mut self, line: &str) {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let method = msg.get("method").and_then(Value::as_str);
        let id = msg.get("id").cloned();

        match (method, id) {
            (Some("initialize"), Some(req_id)) => {
                let resp = self.handle_initialize(&msg, req_id).await;
                let _ = self.client_tx.send(resp.to_string());
            }
            (Some("notifications/initialized"), None) => {}
            (Some("tools/list"), Some(req_id)) => {
                let resp = self.handle_tools_list(req_id).await;
                let _ = self.client_tx.send(resp.to_string());
            }
            (Some("tools/call"), Some(req_id)) => {
                let resp = self.handle_tools_call(msg, req_id).await;
                let _ = self.client_tx.send(resp.to_string());
            }
            (Some(_), Some(req_id)) => {
                let resp = self.forward_generic_request(&msg, req_id).await;
                let _ = self.client_tx.send(resp.to_string());
            }
            (Some(_), None) => {
                if let Some(w) = &self.worker {
                    let _ = w.write_raw(&msg).await;
                }
            }
            _ => {}
        }
    }

    async fn handle_initialize(&mut self, msg: &Value, req_id: Value) -> Value {
        let params = msg.get("params").cloned().unwrap_or_else(|| {
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "proxied-agent-mcp", "version": env!("CARGO_PKG_VERSION") }
            })
        });
        self.init_params = Some(params);

        match self.boot_candidate().await {
            Ok((worker, mut init_result)) => {
                self.worker = Some(worker);
                let version_note = self
                    .current_version_status()
                    .map(|v| format!(" [{}]", v.summary_line()))
                    .unwrap_or_default();

                if let Some(caps) = init_result
                    .get_mut("capabilities")
                    .and_then(Value::as_object_mut)
                {
                    let tools = caps.entry("tools").or_insert_with(|| json!({}));
                    if let Some(tools_obj) = tools.as_object_mut() {
                        tools_obj.insert("listChanged".into(), json!(true));
                    }
                }
                if let Some(obj) = init_result.as_object_mut() {
                    let existing = obj
                        .get("instructions")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let combined = if existing.is_empty() {
                        format!("{PROXY_INSTRUCTION_SUFFIX}{version_note}")
                    } else {
                        format!("{existing} {PROXY_INSTRUCTION_SUFFIX}{version_note}")
                    };
                    obj.insert("instructions".into(), json!(combined));
                }
                json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": init_result
                })
            }
            Err(err) => rpc_error(req_id, -32603, &format!("failed to start MCP worker: {err}")),
        }
    }

    async fn handle_tools_list(&mut self, req_id: Value) -> Value {
        let _ = self.ensure_worker_ready().await;
        let Some(worker) = &self.worker else {
            return rpc_error(req_id, -32603, "no active worker process");
        };
        if self.client_baseline_tools.is_empty() {
            self.client_baseline_tools = worker.tools.clone();
        }
        let mut tools: Vec<Value> = worker
            .tools
            .iter()
            .map(|t| relax_tool_schema(t.raw.clone()))
            .collect();
        for gw in gateway_tool_definitions() {
            if !tools.iter().any(|t| t.get("name") == gw.get("name")) {
                tools.push(gw);
            }
        }
        json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": { "tools": tools }
        })
    }

    async fn handle_tools_call(&mut self, mut msg: Value, req_id: Value) -> Value {
        match self.ensure_worker_ready().await {
            Ok(Some(report)) => self.push_pending_notice(report.format_notice()),
            Ok(None) => {}
            Err(warn) => self.push_pending_notice(warn),
        }

        let Some(params) = msg.get_mut("params").and_then(Value::as_object_mut) else {
            return tool_error_response(req_id, "missing `params` in `tools/call`");
        };
        let Some(tool_name) = params.get("name").and_then(Value::as_str).map(str::to_string) else {
            return tool_error_response(req_id, "missing tool `name` in `tools/call`");
        };
        let raw_args = params.get("arguments").cloned();

        let effective_tool = match tool_name.as_str() {
            RELOAD_TOOL => {
                let force = raw_args
                    .as_ref()
                    .and_then(|a| a.get("force"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                return self.handle_reload_tool(req_id, force).await;
            }
            LIST_TOOLS => {
                let header = self
                    .current_version_status()
                    .map(|v| format!("{}\n", v.summary_line()))
                    .unwrap_or_default();
                let diff = self
                    .worker
                    .as_ref()
                    .map(|w| diff_tools(&self.client_baseline_tools, &w.tools))
                    .unwrap_or_default();
                let live = self
                    .worker
                    .as_ref()
                    .map(|w| w.tools.as_slice())
                    .unwrap_or(&[]);
                let mut text =
                    format!("{header}{}", render_tool_list(live, &diff, raw_args.as_ref()));
                if let Some(notice) = self.pending_notice.take() {
                    text = format!("{text}\n{notice}\n");
                }
                return tool_text_response(req_id, text);
            }
            CALL_TOOL => match unwrap_call_tool(raw_args.as_ref()) {
                Ok((target_name, target_args)) => {
                    params.insert("name".into(), json!(target_name));
                    params.insert("arguments".into(), target_args);
                    target_name
                }
                Err(reason) => return tool_error_response(req_id, &reason),
            },
            _ => tool_name,
        };

        let effective_args = msg
            .get("params")
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or_else(|| json!({}));

        let Some(worker) = &self.worker else {
            return tool_error_response(req_id, "no active worker process");
        };
        let mut resp = match worker.forward_request(&msg, TOOL_CALL_TIMEOUT).await {
            Ok(resp) => resp,
            Err(err) => {
                let recovery = match self.reload_if_needed(true).await {
                    Ok(_) => "worker has been restarted and state replayed".to_string(),
                    Err(e) => format!("worker restart also failed: {e}"),
                };
                return tool_error_response(
                    req_id,
                    &format!("tool `{effective_tool}` failed ({err}); {recovery}"),
                );
            }
        };

        // When `workflow_register` succeeds, update `workspace_root`, resolve turn version
        // (resetting to `01.01` if `sessions/*` was deleted before this new session), and
        // inject `mcp_build` into the registration response.
        let is_error = resp
            .get("result")
            .and_then(|r| r.get("isError"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || resp.get("error").is_some();
        if !is_error && effective_tool == REGISTER_TOOL {
            self.register_args = Some(effective_args);
            if let Some(root) = extract_json_field(&resp, "project_root") {
                self.workspace_root = Some(PathBuf::from(root));
            }
            if let Some(status) = self.current_version_status() {
                inject_build_status(&mut resp, &status);
            }
        }

        // If the tool call rebuilt the binary on disk (e.g. `langtool_run_action` running `cargo build`),
        // hot-reload immediately, stamp the current code turn onto the new binary, and attach the diff.
        let current_fp = self.worker.as_ref().map(|w| &w.fingerprint);
        if self.binaries.has_changed_meta(current_fp) {
            match self.reload_if_needed(false).await {
                Ok(Some(report)) => self.push_pending_notice(report.format_notice()),
                Ok(None) => {}
                Err(err) => self.push_pending_notice(format!(
                    "[proxy warning: binary changed on disk, but reload failed ({err}); keeping previous worker alive]"
                )),
            }
        }

        if let Some(notice) = self.pending_notice.take() {
            append_notice_to_result(&mut resp, &notice);
        }
        resp
    }

    async fn handle_reload_tool(&mut self, req_id: Value, force: bool) -> Value {
        match self.reload_if_needed(force).await {
            Ok(Some(report)) => {
                let mut text = report.format_notice();
                if let Some(prev) = self.pending_notice.take() {
                    text = format!("{prev}\n{text}");
                }
                text.push('\n');
                tool_text_response(req_id, text)
            }
            Ok(None) => {
                let status = self
                    .current_version_status()
                    .map(|v| v.summary_line())
                    .unwrap_or_else(|| "no active worker".into());
                let tools_len = self.worker.as_ref().map(|w| w.tools.len()).unwrap_or(0);
                tool_text_response(
                    req_id,
                    format!(
                        "Binary unchanged ({status}, {tools_len} tools). Pass `force: true` to restart anyway.\n"
                    ),
                )
            }
            Err(err) => tool_error_response(req_id, &format!("reload failed: {err}")),
        }
    }

    async fn forward_generic_request(&mut self, msg: &Value, req_id: Value) -> Value {
        let _ = self.ensure_worker_ready().await;
        let Some(worker) = &self.worker else {
            return rpc_error(req_id, -32603, "no active worker process");
        };
        match worker.forward_request(msg, HANDSHAKE_TIMEOUT).await {
            Ok(resp) => resp,
            Err(err) => rpc_error(req_id, -32603, &err.to_string()),
        }
    }

    async fn ensure_worker_ready(&mut self) -> Result<Option<ReloadReport>, String> {
        let dead = self.worker.as_mut().is_none_or(|w| !w.is_alive());
        let current_fp = self.worker.as_ref().map(|w| &w.fingerprint);
        if dead || self.binaries.has_changed_meta(current_fp) {
            self.reload_if_needed(dead).await.map_err(|err| {
                format!(
                    "[proxy warning: new binary failed startup check ({err}); keeping previous worker alive]"
                )
            })
        } else {
            Ok(None)
        }
    }

    pub async fn reload_if_needed(&mut self, force: bool) -> anyhow::Result<Option<ReloadReport>> {
        let settled_fp = self.binaries.settled_fingerprint().await?;
        let binary_changed = self
            .worker
            .as_ref()
            .is_none_or(|w| w.fingerprint.hash != settled_fp.hash);

        if !force && !binary_changed {
            if let Some(w) = self.worker.as_mut() {
                w.fingerprint = settled_fp;
            }
            return Ok(None);
        }

        let prev_build_turn = self.current_version_status().map(|v| v.worker_turn);
        let (mut new_worker, _) = self.boot_candidate().await?;

        // Stamp the newly compiled binary with the workspace's current code turn.
        let new_build_turn = match (&self.workspace_root, binary_changed) {
            (Some(root), true) => self.binaries.record_build_turn(&new_worker.fingerprint, root),
            (root, _) => {
                self.binaries
                    .resolve_version_status(&new_worker.fingerprint, root.as_deref())
                    .worker_turn
            }
        };

        let active_turn_id = self.replay_register(&mut new_worker).await;

        let prev_tools = self
            .worker
            .as_ref()
            .map(|w| w.tools.as_slice())
            .unwrap_or(&self.client_baseline_tools);
        let diff_vs_prev = diff_tools(prev_tools, &new_worker.tools);
        let diff_vs_client = diff_tools(&self.client_baseline_tools, &new_worker.tools);

        let report = ReloadReport {
            prev_build_turn,
            new_build_turn,
            active_turn_id,
            diff_vs_prev,
            diff_vs_client,
        };

        if let Some(mut old) = self.worker.replace(new_worker) {
            let old_staged = old.staged_path().to_path_buf();
            old.kill().await;
            self.binaries.remove_staged(&old_staged);
        }

        let list_changed = json!({
            "jsonrpc": "2.0",
            "method": "notifications/tools/list_changed"
        });
        let _ = self.client_tx.send(list_changed.to_string());

        Ok(Some(report))
    }

    async fn boot_candidate(&mut self) -> anyhow::Result<(Worker, Value)> {
        let fp = self.binaries.settled_fingerprint().await?;
        let generation = self.next_generation;
        self.next_generation += 1;

        let staged = self.binaries.stage(&fp, generation)?;
        let mut candidate = Worker::spawn(
            generation,
            staged.clone(),
            &self.worker_args,
            fp,
            self.client_tx.clone(),
        )?;

        let init_params = self.init_params.clone().unwrap_or_else(|| {
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "proxied-agent-mcp", "version": env!("CARGO_PKG_VERSION") }
            })
        });

        let handshake = async {
            let init_res = candidate
                .call_internal("initialize", Some(init_params), HANDSHAKE_TIMEOUT)
                .await?;
            candidate
                .send_notification("notifications/initialized", None)
                .await?;
            let tools_res = candidate
                .call_internal("tools/list", Some(json!({})), HANDSHAKE_TIMEOUT)
                .await?;
            candidate.tools = parse_tools(&tools_res);
            Ok::<Value, anyhow::Error>(init_res)
        }
        .await;

        match handshake {
            Ok(init_res) => Ok((candidate, init_res)),
            Err(err) => {
                candidate.kill().await;
                self.binaries.remove_staged(&staged);
                Err(err)
            }
        }
    }

    /// Always replay `workflow_register` on worker reload, returning the resulting `active_turn_id`.
    async fn replay_register(&self, worker: &mut Worker) -> Option<String> {
        let mut replay_args = self.register_args.clone()?;
        if let Some(obj) = replay_args.as_object_mut() {
            obj.insert("show_instructions".into(), json!(false));
            obj.insert("show_last_turn_info".into(), json!(false));
        }
        let params = json!({
            "name": REGISTER_TOOL,
            "arguments": replay_args,
        });
        let res = worker
            .call_internal("tools/call", Some(params), HANDSHAKE_TIMEOUT)
            .await
            .ok()?;
        let text = res
            .get("content")?
            .as_array()?
            .first()?
            .get("text")?
            .as_str()?;
        let parsed: Value = serde_json::from_str(text).ok()?;
        parsed
            .get("active_turn_id")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    fn push_pending_notice(&mut self, notice: String) {
        match &mut self.pending_notice {
            Some(existing) => {
                existing.push('\n');
                existing.push_str(&notice);
            }
            None => self.pending_notice = Some(notice),
        }
    }

    pub async fn shutdown(&mut self) {
        if let Some(mut w) = self.worker.take() {
            w.kill().await;
        }
        self.binaries.cleanup_all();
    }
}

fn extract_json_field(resp: &Value, field: &str) -> Option<String> {
    let text = resp
        .get("result")?
        .get("content")?
        .as_array()?
        .first()?
        .get("text")?
        .as_str()?;
    let parsed: Value = serde_json::from_str(text).ok()?;
    parsed.get(field).and_then(Value::as_str).map(str::to_string)
}

fn inject_build_status(resp: &mut Value, status: &BuildVersionStatus) {
    let Some(first) = resp
        .get_mut("result")
        .and_then(|r| r.get_mut("content"))
        .and_then(Value::as_array_mut)
        .and_then(|a| a.first_mut())
    else {
        return;
    };
    let Some(text) = first.get("text").and_then(Value::as_str) else {
        return;
    };
    if let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(text) {
        map.insert("mcp_build".into(), status.to_json());
        first["text"] = json!(Value::Object(map).to_string());
    }
}

fn append_notice_to_result(resp: &mut Value, notice: &str) {
    let Some(content) = resp
        .get_mut("result")
        .and_then(|r| r.get_mut("content"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    if let Some(first) = content.first_mut()
        && let Some(text) = first.get("text").and_then(Value::as_str).map(str::to_string)
    {
        if let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(&text) {
            map.insert("proxy_notice".into(), json!(notice));
            first["text"] = json!(Value::Object(map).to_string());
            return;
        }
        let sep = if text.ends_with('\n') { "" } else { "\n" };
        first["text"] = json!(format!("{text}{sep}{notice}\n"));
        return;
    }
    content.push(json!({
        "type": "text",
        "text": format!("{notice}\n")
    }));
}

fn tool_text_response(id: Value, text: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": text }],
            "isError": false
        }
    })
}

fn tool_error_response(id: Value, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": message }],
            "isError": true
        }
    })
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}