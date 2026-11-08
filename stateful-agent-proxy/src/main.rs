//! Drop-in hot-reloading stdio proxy for `stateful-agent`.
//!
//! Resolves the sibling `stateful-agent` binary in the same directory as this executable and
//! forwards all CLI arguments (`--mode`, `--max-output-bytes`, `[PROJECT]`, etc.) directly to it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};

use stateful_agent_proxy::router::ProxyRouter;

const POLL_MS: u64 = 500;

/// Locate the target MCP server binary in the same directory as this proxy executable and
/// collect all CLI arguments to forward to it.
fn resolve_sibling_target() -> anyhow::Result<(PathBuf, Vec<String>)> {
    let self_exe = std::env::current_exe()?;
    let dir = self_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine executable directory"))?;
    let target = dir.join(format!("stateful-agent{}", std::env::consts::EXE_SUFFIX));

    let forwarded_args: Vec<String> = std::env::args().skip(1).collect();
    Ok((target, forwarded_args))
}

/// Infer the bootstrapped workspace root at startup: either from a positional `[PROJECT]` CLI
/// argument or by walking up from `<workspace>/target/<profile>/<exe>` to the directory
/// containing `.AGENTS`.
fn infer_bootstrap_workspace(target_bin: &Path, args: &[String]) -> Option<PathBuf> {
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if matches!(
            arg.as_str(),
            "--mode" | "--max-output-bytes" | "--default-limit" | "--max-dir-depth"
        ) {
            skip_next = true;
            continue;
        }
        if !arg.starts_with('-') {
            let path = PathBuf::from(arg);
            if path.is_dir() {
                return std::fs::canonicalize(&path).ok().or(Some(path));
            }
        }
    }
    for ancestor in target_bin.ancestors().skip(1).take(4) {
        if ancestor.join(".AGENTS").is_dir() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (target_bin, worker_args) = resolve_sibling_target()?;
    let bootstrap_workspace = infer_bootstrap_workspace(&target_bin, &worker_args);
    let (client_tx, mut client_rx) = mpsc::unbounded_channel::<String>();
    let router = Arc::new(Mutex::new(ProxyRouter::new(
        target_bin,
        worker_args,
        bootstrap_workspace,
        client_tx,
    )?));

    // Forward outgoing JSON-RPC messages to the MCP client's stdout.
    let stdout_task = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(line) = client_rx.recv().await {
            if stdout.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if !line.ends_with('\n') && stdout.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = stdout.flush().await;
        }
    });

    // Background watcher: detects external `cargo build` runs while idle.
    let watch_router = Arc::clone(&router);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(POLL_MS));
        loop {
            interval.tick().await;
            if let Ok(mut guard) = watch_router.try_lock() {
                guard.poll_background_reload().await;
            }
        }
    });

    // Read newline-delimited JSON-RPC messages from the MCP client's stdin.
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut guard = router.lock().await;
        guard.handle_client_line(trimmed).await;
    }

    router.lock().await.shutdown().await;
    let _ = stdout_task.await;
    Ok(())
}
