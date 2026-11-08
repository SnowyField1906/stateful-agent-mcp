//! Disk persistence for `.AGENTS/sessions/*.md`, `.AGENTS/.deleted/{SS.TT}/*` and
//! `.AGENTS/.known/*`.
//! All writes are atomic (temporary file + rename).

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{DefaultHasher, Hasher};
use std::io::ErrorKind;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::common::io::atomic_write;
use crate::common::{AgentError, Result};

use super::model::{TurnKey, TurnRecord};
use super::parse::parse;
use super::serialize::serialize;

/// Pre-turn content of files deleted in a turn, keyed by project-relative path.
pub type DeletedMap = BTreeMap<String, String>;

pub fn load_sessions(dir: &Path) -> Result<BTreeMap<u32, Vec<TurnRecord>>> {
    let mut sessions = BTreeMap::new();
    for entry in std::fs::read_dir(dir).map_err(|e| AgentError::io_at(dir, e))? {
        let path = entry.map_err(|e| AgentError::io_at(dir, e))?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(number) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let text = std::fs::read_to_string(&path).map_err(|e| AgentError::io_at(&path, e))?;
        let mut turns = parse(number, &text, &path.to_string_lossy())?;
        if turns.is_empty() {
            turns.push(TurnRecord::new(TurnKey::new(number, 1)));
        }
        sessions.insert(number, turns);
    }
    Ok(sessions)
}

pub fn save_session(dir: &Path, session: u32, turns: &[TurnRecord]) -> Result<()> {
    atomic_write(
        &dir.join(format!("{session:02}.md")),
        serialize(turns).as_bytes(),
    )
}

/// Load deleted file contents stored under `.AGENTS/.deleted/{SS.TT}/{rel_path}`.
pub fn load_deleted(dir: &Path) -> Result<BTreeMap<TurnKey, DeletedMap>> {
    let mut out = BTreeMap::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(AgentError::io_at(dir, e)),
    };
    for entry in entries {
        let path = entry.map_err(|e| AgentError::io_at(dir, e))?.path();
        if !path.is_dir() {
            continue;
        }
        let Some(key) = path
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(TurnKey::parse)
        else {
            continue;
        };
        let mut map = DeletedMap::new();
        collect_deleted_files(&path, "", &mut map)?;
        if !map.is_empty() {
            out.insert(key, map);
        }
    }
    Ok(out)
}

fn collect_deleted_files(dir: &Path, prefix: &str, out: &mut DeletedMap) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| AgentError::io_at(dir, e))? {
        let entry = entry.map_err(|e| AgentError::io_at(dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".tmp") {
            continue;
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let path = entry.path();
        let ty = entry.file_type().map_err(|e| AgentError::io_at(&path, e))?;
        if ty.is_dir() {
            collect_deleted_files(&path, &rel, out)?;
        } else if ty.is_file() {
            let text = std::fs::read_to_string(&path).map_err(|e| AgentError::io_at(&path, e))?;
            out.insert(rel, text);
        }
    }
    Ok(())
}

/// Synchronize `.AGENTS/.deleted/{key}/` on disk with `new`, removing any files that were
/// restored within the turn and pruning empty directories.
pub fn sync_turn_deleted(
    dir: &Path,
    key: TurnKey,
    old: Option<&DeletedMap>,
    new: &DeletedMap,
) -> Result<()> {
    let turn_dir = dir.join(key.to_string());
    if let Some(old_map) = old {
        for rel in old_map.keys() {
            if !new.contains_key(rel) {
                let file_path = turn_dir.join(rel);
                match std::fs::remove_file(&file_path) {
                    Ok(()) => prune_empty_dirs(&file_path, dir),
                    Err(e) if e.kind() == ErrorKind::NotFound => {}
                    Err(e) => return Err(AgentError::io_at(&file_path, e)),
                }
            }
        }
    }
    for (rel, content) in new {
        if old.and_then(|m| m.get(rel)) == Some(content) {
            continue;
        }
        let file_path = turn_dir.join(rel);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AgentError::io_at(parent, e))?;
        }
        atomic_write(&file_path, content.as_bytes())?;
    }
    Ok(())
}

/// Remove `.AGENTS/.deleted/{key}/` if it exists.
pub fn remove_turn_deleted(dir: &Path, key: TurnKey) -> Result<()> {
    let turn_dir = dir.join(key.to_string());
    match std::fs::remove_dir_all(&turn_dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AgentError::io_at(&turn_dir, e)),
    }
}

/// Paths of the files kept under `.AGENTS/.known/`, sorted. Each holds the content the ledger
/// last saw of the project file at that path.
pub fn known_paths(dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    collect_known_paths(dir, "", &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_known_paths(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(AgentError::io_at(dir, e)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| AgentError::io_at(dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".tmp") {
            continue;
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let path = entry.path();
        let ty = entry.file_type().map_err(|e| AgentError::io_at(&path, e))?;
        if ty.is_dir() {
            collect_known_paths(&path, &rel, out)?;
        } else if ty.is_file() {
            out.push(rel);
        }
    }
    Ok(())
}

/// The content the ledger last saw of the project file `rel`; `None` when no copy is kept.
pub fn read_known(dir: &Path, rel: &str) -> Result<Option<String>> {
    let path = dir.join(rel);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AgentError::io_at(&path, e)),
    }
}

/// Keep `content` as what the ledger last saw of `rel`; `None` drops the copy and the
/// directories it leaves empty.
pub fn write_known(dir: &Path, rel: &str, content: Option<&str>) -> Result<()> {
    let path = dir.join(rel);
    match content {
        Some(text) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AgentError::io_at(parent, e))?;
            }
            atomic_write(&path, text.as_bytes())
        }
        None => match std::fs::remove_file(&path) {
            Ok(()) => {
                prune_empty_dirs(&path, dir);
                Ok(())
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AgentError::io_at(&path, e)),
        },
    }
}

/// Remove the directories above `file` that are empty, up to (not including) `stop_at`.
pub fn prune_empty_dirs(file: &Path, stop_at: &Path) {
    let mut cur = file.parent();
    while let Some(d) = cur {
        if d == stop_at || !d.starts_with(stop_at) || std::fs::remove_dir(d).is_err() {
            break;
        }
        cur = d.parent();
    }
}

/// 64-bit non-cryptographic content hash fingerprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(pub u64);

impl ContentHash {
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl From<u64> for ContentHash {
    fn from(val: u64) -> Self {
        Self::new(val)
    }
}

impl From<ContentHash> for u64 {
    fn from(h: ContentHash) -> Self {
        h.0
    }
}

/// Fast metadata fingerprint for a known file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownEntry {
    pub size: u64,
    pub mtime: u64,
    pub hash: ContentHash,
}

pub type KnownIndex = BTreeMap<String, KnownEntry>;

/// Fast 64-bit non-cryptographic content hash.
pub fn content_hash(bytes: &[u8]) -> ContentHash {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    ContentHash::new(hasher.finish())
}

/// Nanoseconds since Unix epoch.
pub fn meta_mtime(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

pub fn load_known_index(file: &Path, known_dir: &Path, root: &Path) -> Result<KnownIndex> {
    if !known_dir.exists() {
        return Ok(KnownIndex::new());
    }
    if let Ok(text) = std::fs::read_to_string(file) {
        if let Ok(index) = serde_json::from_str::<KnownIndex>(&text) {
            return Ok(index);
        }
    }
    let paths = known_paths(known_dir)?;
    let mut index = KnownIndex::new();
    for rel in paths {
        let disk_path = root.join(&rel);
        let known_path = known_dir.join(&rel);
        if let Ok(meta) = std::fs::symlink_metadata(&disk_path) {
            let size = meta.len();
            let mtime = meta_mtime(&meta);
            let bytes = std::fs::read(&known_path).unwrap_or_default();
            let hash = content_hash(&bytes);
            index.insert(rel, KnownEntry { size, mtime, hash });
        } else if let Ok(meta) = std::fs::symlink_metadata(&known_path) {
            let size = meta.len();
            let bytes = std::fs::read(&known_path).unwrap_or_default();
            let hash = content_hash(&bytes);
            index.insert(
                rel,
                KnownEntry {
                    size,
                    mtime: 0,
                    hash,
                },
            );
        }
    }
    let _ = save_known_index(file, &index);
    Ok(index)
}

pub fn save_known_index(file: &Path, index: &KnownIndex) -> Result<()> {
    let text = serde_json::to_string(index)
        .map_err(|e| AgentError::invalid("known_index", e.to_string()))?;
    atomic_write(file, text.as_bytes())
}
