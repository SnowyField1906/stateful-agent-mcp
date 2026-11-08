//! Binary fingerprinting, copy-on-exec staging, and turn-based build version tracking.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SETTLE_MS: u64 = 250;

/// Identity of the compiled binary on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_nanos: u128,
    pub hash: u64,
}

impl Fingerprint {
    pub fn hash_key(&self) -> String {
        format!("{:016x}", self.hash)
    }
}

/// Watches the sibling MCP binary in the same directory and copies each generation to a
/// temporary file before spawning so `cargo build` never locks the executable.
pub struct BinaryManager {
    target: PathBuf,
    staging_dir: PathBuf,
    turns_cache_path: PathBuf,
}

impl BinaryManager {
    pub fn new(target: PathBuf) -> anyhow::Result<Self> {
        let staging_dir =
            std::env::temp_dir().join(format!("mcp-proxy-staging-{}", std::process::id()));
        std::fs::create_dir_all(&staging_dir)?;
        let turns_cache_path = std::env::temp_dir().join("mcp-proxy-build-turns.json");
        Ok(Self {
            target,
            staging_dir,
            turns_cache_path,
        })
    }

    /// Fast metadata check: returns `true` when the target binary on disk differs in size or
    /// modification timestamp from `current`.
    pub fn has_changed_meta(&self, current: Option<&Fingerprint>) -> bool {
        let Some(current) = current else {
            return true;
        };
        let Ok(meta) = std::fs::metadata(&self.target) else {
            return false;
        };
        meta.len() != current.size || mtime_nanos(&meta) != current.mtime_nanos
    }

    /// Wait until the target binary stops changing on disk (linker finished) and compute its
    /// content fingerprint.
    pub async fn settled_fingerprint(&self) -> anyhow::Result<Fingerprint> {
        if !self.target.is_file() {
            anyhow::bail!("target MCP binary not found at `{}`", self.target.display());
        }
        let mut prev = meta_pair(&self.target)?;
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
            let now = meta_pair(&self.target)?;
            if now == prev && now.0 > 0 {
                break;
            }
            prev = now;
        }
        fingerprint(&self.target)
    }

    /// Copy `fp.path` to a generation-specific staging path so the original binary is never
    /// locked while the worker runs.
    pub fn stage(&self, fp: &Fingerprint, generation: u64) -> anyhow::Result<PathBuf> {
        let stem = fp
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("worker");
        let staged = self.staging_dir.join(format!(
            "{stem}-gen-{generation}{}",
            std::env::consts::EXE_SUFFIX
        ));
        std::fs::copy(&fp.path, &staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&staged)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&staged, perms)?;
        }
        Ok(staged)
    }

    pub fn remove_staged(&self, path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    pub fn cleanup_all(&self) {
        let _ = std::fs::remove_dir_all(&self.staging_dir);
    }

    /// Record that `fp` was built when `workspace_root` was at `turn_id`.
    pub fn record_build_turn(&self, fp: &Fingerprint, workspace_root: &Path) -> String {
        let turns = load_workspace_turns(workspace_root);
        let turn_id = latest_code_turn(&turns);
        self.save_cached_turn(fp, workspace_root, &turn_id);
        turn_id
    }

    /// Resolve the turn version of `fp` and compare it against `workspace_root`'s current turns.
    ///
    /// - If `.AGENTS/sessions/*` was deleted or is a fresh session (`01.01` with no updates),
    ///   resets the binary's recorded turn to `01.01` so it points directly to the first turn.
    /// - If the recorded turn no longer exists in `.AGENTS/sessions/*`, re-infers it from the
    ///   remaining turns (or `01.01`).
    pub fn resolve_version_status(
        &self,
        fp: &Fingerprint,
        workspace_root: Option<&Path>,
    ) -> BuildVersionStatus {
        let Some(root) = workspace_root else {
            return BuildVersionStatus {
                worker_turn: "01.01".into(),
                latest_code_turn: "01.01".into(),
                active_turn: "01.01".into(),
                up_to_date: true,
            };
        };

        let turns = load_workspace_turns(root);
        let first_turn = turns
            .first()
            .map(|t| t.id.clone())
            .unwrap_or_else(|| "01.01".into());
        let active_turn = turns
            .last()
            .map(|t| t.id.clone())
            .unwrap_or_else(|| first_turn.clone());
        let latest_code = latest_code_turn(&turns);

        // If `sessions/*` is fresh/empty, point directly to the first turn (`01.01`).
        let all_empty = turns.iter().all(|t| t.is_empty);
        if all_empty {
            self.save_cached_turn(fp, root, &first_turn);
            return BuildVersionStatus {
                worker_turn: first_turn.clone(),
                latest_code_turn: first_turn,
                active_turn,
                up_to_date: true,
            };
        }

        let valid_cached = self
            .load_cached_turn(fp, root)
            .filter(|saved| turns.iter().any(|t| &t.id == saved));

        let worker_turn = match valid_cached {
            Some(saved) => saved,
            None => {
                let inferred = infer_turn_from_mtime(fp.mtime_nanos, root, &turns, &first_turn);
                self.save_cached_turn(fp, root, &inferred);
                inferred
            }
        };

        let up_to_date = turn_key(&worker_turn) >= turn_key(&latest_code);
        BuildVersionStatus {
            worker_turn,
            latest_code_turn: latest_code,
            active_turn,
            up_to_date,
        }
    }

    fn load_cached_turn(&self, fp: &Fingerprint, root: &Path) -> Option<String> {
        let text = std::fs::read_to_string(&self.turns_cache_path).ok()?;
        let map: BTreeMap<String, CachedBuild> = serde_json::from_str(&text).ok()?;
        let entry = map.get(&fp.hash_key())?;
        (Path::new(&entry.root) == root).then(|| entry.turn_id.clone())
    }

    fn save_cached_turn(&self, fp: &Fingerprint, root: &Path, turn_id: &str) {
        let mut map: BTreeMap<String, CachedBuild> =
            std::fs::read_to_string(&self.turns_cache_path)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
        map.insert(
            fp.hash_key(),
            CachedBuild {
                root: root.to_string_lossy().into_owned(),
                turn_id: turn_id.to_string(),
            },
        );
        if let Ok(json) = serde_json::to_string(&map) {
            let _ = std::fs::write(&self.turns_cache_path, json);
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CachedBuild {
    root: String,
    turn_id: String,
}

/// Comparison between the running MCP binary's build turn and the workspace's current turns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildVersionStatus {
    /// Turn ID (`SS.TT`) when the currently running MCP binary was built.
    pub worker_turn: String,
    /// Latest turn ID (`SS.TT`) in `.AGENTS/sessions/` that modified files (or `01.01` when fresh).
    pub latest_code_turn: String,
    /// Current active turn ID (`SS.TT`) in `.AGENTS/sessions/`.
    pub active_turn: String,
    /// True when `worker_turn >= latest_code_turn`.
    pub up_to_date: bool,
}

impl BuildVersionStatus {
    pub fn summary_line(&self) -> String {
        if self.up_to_date {
            format!(
                "mcp_build_turn={} (up-to-date; active_turn={})",
                self.worker_turn, self.active_turn
            )
        } else {
            format!(
                "mcp_build_turn={} (OUTDATED: latest code changes at turn {}; active_turn={})",
                self.worker_turn, self.latest_code_turn, self.active_turn
            )
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "worker_turn": self.worker_turn,
            "latest_code_turn": self.latest_code_turn,
            "active_turn": self.active_turn,
            "up_to_date": self.up_to_date,
        })
    }
}

/// One turn of a session file as the proxy reads it.
#[derive(Clone, Debug)]
pub struct TurnSummary {
    pub id: String,
    pub session_mtime_nanos: u128,
    pub updated_paths: Vec<String>,
    pub is_empty: bool,
}

fn latest_code_turn(turns: &[TurnSummary]) -> String {
    turns
        .iter()
        .rev()
        .find(|t| !t.updated_paths.is_empty())
        .or_else(|| turns.iter().rev().find(|t| !t.is_empty))
        .or_else(|| turns.first())
        .map(|t| t.id.clone())
        .unwrap_or_else(|| "01.01".into())
}

/// Infer which turn a binary with `bin_mtime` was built at by comparing `bin_mtime` against
/// session files and the files touched by each turn.
fn infer_turn_from_mtime(
    bin_mtime: u128,
    root: &Path,
    turns: &[TurnSummary],
    first_turn: &str,
) -> String {
    let mut matched = first_turn.to_string();
    for turn in turns {
        if turn.updated_paths.is_empty() && turn.is_empty {
            continue;
        }
        let turn_mtime = if turn.session_mtime_nanos <= bin_mtime {
            turn.session_mtime_nanos
        } else {
            // Session file was updated later; check the mtime of files this turn touched.
            turn.updated_paths
                .iter()
                .filter_map(|rel| std::fs::metadata(root.join(rel)).ok())
                .map(|m| mtime_nanos(&m))
                .max()
                .unwrap_or(turn.session_mtime_nanos)
        };
        if turn_mtime <= bin_mtime {
            matched = turn.id.clone();
        }
    }
    matched
}

/// Read all turns in `<root>/.AGENTS/sessions/*.md` in chronological order.
pub fn load_workspace_turns(root: &Path) -> Vec<TurnSummary> {
    let sessions_dir = root.join(".AGENTS").join("sessions");
    let Ok(read_dir) = std::fs::read_dir(&sessions_dir) else {
        return vec![TurnSummary {
            id: "01.01".into(),
            session_mtime_nanos: 0,
            updated_paths: Vec::new(),
            is_empty: true,
        }];
    };

    let mut session_files: Vec<(u32, PathBuf, u128)> = read_dir
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                return None;
            }
            let num = path.file_stem()?.to_str()?.parse::<u32>().ok()?;
            let mtime = entry.metadata().ok().map(|m| mtime_nanos(&m)).unwrap_or(0);
            Some((num, path, mtime))
        })
        .collect();
    session_files.sort_by_key(|(num, _, _)| *num);

    let mut turns = Vec::new();
    for (_, path, session_mtime_nanos) in session_files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        turns.extend(parse_session_turns(&text, session_mtime_nanos));
    }

    if turns.is_empty() {
        turns.push(TurnSummary {
            id: "01.01".into(),
            session_mtime_nanos: 0,
            updated_paths: Vec::new(),
            is_empty: true,
        });
    }
    turns
}

fn parse_session_turns(markdown: &str, session_mtime_nanos: u128) -> Vec<TurnSummary> {
    let mut out: Vec<TurnSummary> = Vec::new();
    for line in markdown.lines() {
        if let Some(rest) = line.strip_prefix("## TURN ") {
            let rest = rest.trim();
            let (id, has_title) = match rest.split_once(" — ") {
                Some((id, title)) => (id.trim(), !title.trim().is_empty()),
                None => (rest, false),
            };
            if turn_key(id).is_some() {
                out.push(TurnSummary {
                    id: id.to_string(),
                    session_mtime_nanos,
                    updated_paths: Vec::new(),
                    is_empty: !has_title,
                });
            }
            continue;
        }
        let Some(cur) = out.last_mut() else {
            continue;
        };
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with("### ") {
            cur.is_empty = false;
        }
        if let Some(summary) = trimmed
            .strip_prefix("<summary>")
            .and_then(|s| s.strip_suffix("</summary>"))
            && let Some((_, after)) = summary.split_once("<code>")
            && let Some((path, _)) = after.split_once("</code>")
        {
            cur.updated_paths.push(path.to_string());
        }
    }
    out
}

fn turn_key(id: &str) -> Option<(u32, u32)> {
    let (s, t) = id.trim().split_once('.')?;
    Some((s.parse().ok()?, t.parse().ok()?))
}

fn meta_pair(path: &Path) -> anyhow::Result<(u64, u128)> {
    let meta = std::fs::metadata(path)?;
    Ok((meta.len(), mtime_nanos(&meta)))
}

fn mtime_nanos(meta: &std::fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

/// Compute a 64-bit FNV-1a hash and metadata fingerprint of `path`.
pub fn fingerprint(path: &Path) -> anyhow::Result<Fingerprint> {
    let meta = std::fs::metadata(path)?;
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() {
        anyhow::bail!("binary `{}` is empty", path.display());
    }
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in &bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Ok(Fingerprint {
        path: path.to_path_buf(),
        size: meta.len(),
        mtime_nanos: mtime_nanos(&meta),
        hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resets_to_first_turn_when_sessions_deleted_and_detects_outdated_turn() {
        let dir = std::env::temp_dir().join(format!("mcp-proxy-ver-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sessions = dir.join(".AGENTS/sessions");
        std::fs::create_dir_all(&sessions).unwrap();

        let bin = dir.join("stateful-agent");
        std::fs::write(&bin, b"binary-v1").unwrap();
        let mgr = BinaryManager::new(bin.clone()).unwrap();
        let fp = fingerprint(&bin).unwrap();

        // 1. Built at turn 01.02.
        std::fs::write(
            sessions.join("01.md"),
            "## TURN 01.01 — First\n\n### Updates\n\n<details>\n<summary>Create <code>src/a.rs</code> (+1)</summary>\n</details>\n\n## TURN 01.02 — Second\n\n### Updates\n\n<details>\n<summary>Patch <code>src/a.rs</code> (+1 -1)</summary>\n</details>\n",
        )
        .unwrap();
        assert_eq!(mgr.record_build_turn(&fp, &dir), "01.02");
        let status = mgr.resolve_version_status(&fp, Some(&dir));
        assert_eq!(status.worker_turn, "01.02");
        assert!(status.up_to_date);

        // 2. Workspace advances to turn 01.03 without rebuilding binary -> outdated!
        std::fs::write(
            sessions.join("01.md"),
            "## TURN 01.01 — First\n\n### Updates\n\n<details>\n<summary>Create <code>src/a.rs</code> (+1)</summary>\n</details>\n\n## TURN 01.02 — Second\n\n### Updates\n\n<details>\n<summary>Patch <code>src/a.rs</code> (+1 -1)</summary>\n</details>\n\n## TURN 01.03 — Third\n\n### Updates\n\n<details>\n<summary>Patch <code>src/a.rs</code> (+2 -1)</summary>\n</details>\n",
        )
        .unwrap();
        let status = mgr.resolve_version_status(&fp, Some(&dir));
        assert_eq!(status.worker_turn, "01.02");
        assert_eq!(status.latest_code_turn, "01.03");
        assert!(!status.up_to_date);

        // 3. User deletes `sessions/*` and starts a new session (`01.01` empty) -> points to `01.01`.
        std::fs::remove_file(sessions.join("01.md")).unwrap();
        std::fs::write(sessions.join("01.md"), "## TURN 01.01\n").unwrap();
        let status = mgr.resolve_version_status(&fp, Some(&dir));
        assert_eq!(status.worker_turn, "01.01");
        assert_eq!(status.latest_code_turn, "01.01");
        assert!(status.up_to_date);

        mgr.cleanup_all();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
