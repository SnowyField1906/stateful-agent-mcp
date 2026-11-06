//! Child worker process supervision and async JSON-RPC request/response multiplexing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::binary::Fingerprint;
use crate::schema::ToolInfo;

pub struct Worker {
    pub generation: u64,
    pub staged_path: PathBuf,
    pub fingerprint: Fingerprint,
    pub tools: Vec<ToolInfo>,
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    seq: AtomicU64,
}

impl Worker {
    pub fn spawn(
        generation: u64,
        staged_path: PathBuf,
        args: &[String],
        fingerprint: Fingerprint,
        client_tx: mpsc::UnboundedSender<String>,
    ) -> anyhow::Result<Self> {
        let mut cmd = Command::new(&staged_path);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("worker stdin was not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("worker stdout was not piped"))?;

        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = Arc::clone(&pending);

        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                    continue;
                };
                if let Some(id_key) = msg.get("id").map(id_to_key) {
                    let tx = reader_pending.lock().await.remove(&id_key);
                    if let Some(tx) = tx {
                        let _ = tx.send(msg);
                        continue;
                    }
                    // Prevent internal responses from leaking to the client if the receiver was dropped.
                    if id_key.starts_with("s:__mcp_proxy_internal_") {
                        continue;
                    }
                }
                let _ = client_tx.send(trimmed.to_string());
            }
            reader_pending.lock().await.clear();
        });

        Ok(Self {
            generation,
            staged_path,
            fingerprint,
            tools: Vec::new(),
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            pending,
            seq: AtomicU64::new(1),
        })
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn call_internal(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        let id_val = json!(format!(
            "__mcp_proxy_internal_{}_{}",
            self.generation,
            self.seq.fetch_add(1, Ordering::Relaxed)
        ));
        let mut req = json!({
            "jsonrpc": "2.0",
            "id": id_val,
            "method": method,
        });
        if let Some(p) = params {
            req["params"] = p;
        }
        let resp = self.send_and_wait(&id_val, &req, timeout).await?;
        if let Some(err) = resp.get("error") {
            anyhow::bail!("worker `{method}` error: {err}");
        }
        resp.get("result")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("worker `{method}` response missing `result`"))
    }

    pub async fn forward_request(&self, msg: &Value, timeout: Duration) -> anyhow::Result<Value> {
        let id_val = msg
            .get("id")
            .ok_or_else(|| anyhow::anyhow!("request has no `id`"))?;
        self.send_and_wait(id_val, msg, timeout).await
    }

    pub async fn send_notification(&self, method: &str, params: Option<Value>) -> anyhow::Result<()> {
        let mut msg = json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        if let Some(p) = params {
            msg["params"] = p;
        }
        self.write_raw(&msg).await
    }

    pub async fn write_raw(&self, msg: &Value) -> anyhow::Result<()> {
        let line = format!("{msg}\n");
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn send_and_wait(
        &self,
        id_val: &Value,
        msg: &Value,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        let id_key = id_to_key(id_val);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id_key.clone(), tx);
        if let Err(err) = self.write_raw(msg).await {
            self.pending.lock().await.remove(&id_key);
            return Err(err);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => anyhow::bail!("worker process exited before replying"),
            Err(_) => {
                self.pending.lock().await.remove(&id_key);
                anyhow::bail!("worker timed out after {}s", timeout.as_secs())
            }
        }
    }

    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }

    pub fn staged_path(&self) -> &Path {
        &self.staged_path
    }
}

fn id_to_key(id: &Value) -> String {
    match id {
        Value::String(s) => format!("s:{s}"),
        Value::Number(n) => format!("n:{n}"),
        other => format!("o:{other}"),
    }
}