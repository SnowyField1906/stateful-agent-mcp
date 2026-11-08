//! A loaded project: sessions, `.AGENTS/.deleted/` records, and reconstruction of past file
//! contents by reverse-applying turn updates.
//!
//! Layout under `<root>/.AGENTS/`:
//! - `INSTRUCTION*.md` — protocol texts, seeded once and then owned by the project
//! - `sessions/{SS}.md` — turns, notes, net file updates, handover tasks
//! - `.deleted/{SS.TT}/{path}` — pre-turn content of files deleted in turn `SS.TT`. Any past
//!   state is rebuilt from current disk files, turn updates, and `.deleted/`.
//! - `.known/{path}` — the content the ledger last saw of each file it has touched, to notice
//!   edits made outside it (see `external`)

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::common::pathspec::AGENTS_DIR;
use crate::common::{AgentError, AgentUseMode, Result, Workspace};
use crate::toolchain::LanguageConfig;

use super::instructions as text;
use super::ops::overlay::read_text_opt;
use super::ops::patch::apply_hunks_reverse;
use super::ops::{FileOp, PatchHunk, Tree};
use super::selector::Sel;
use super::session::model::{TurnKey, TurnRecord};
use super::session::store::{self, DeletedMap};

pub struct Project {
    pub(crate) root: PathBuf,
    pub(crate) agents_dir: PathBuf,
    pub(crate) sessions_dir: PathBuf,
    pub(crate) deleted_dir: PathBuf,
    pub(crate) known_dir: PathBuf,
    pub(crate) known_index_file: PathBuf,
    pub(crate) known_index: store::KnownIndex,
    pub(crate) sessions: BTreeMap<u32, Vec<TurnRecord>>,
    pub(crate) deleted: BTreeMap<TurnKey, DeletedMap>,
    /// Pre-turn baseline of files touched so far in the active turn.
    pub(crate) active_baseline: Tree,
    /// Move origins (`to_path -> pre_turn_from_path`) recorded in the active turn.
    pub(crate) active_moves: BTreeMap<String, String>,
    /// The only writable turn; always the last turn of the last session.
    pub(crate) active: TurnKey,
}

impl Project {
    /// Validate and canonicalize `path` as a project root directory.
    pub fn canonical_root(path: &str) -> Result<PathBuf> {
        let given = PathBuf::from(path.trim());
        if !given.is_absolute() {
            return Err(AgentError::invalid(
                "folder_path",
                "must be an absolute path",
            ));
        }
        let root = std::fs::canonicalize(&given).map_err(|e| AgentError::io_at(&given, e))?;
        if !root.is_dir() {
            return Err(AgentError::invalid("folder_path", "not a directory"));
        }
        Ok(root)
    }

    /// Load `path`, create `.AGENTS/` if needed, and begin a session. The most recent session is
    /// reused when it has recorded nothing.
    pub fn open(path: &str) -> Result<Self> {
        let root = Self::canonical_root(path)?;
        let agents_dir = root.join(AGENTS_DIR);
        let sessions_dir = agents_dir.join("sessions");
        let deleted_dir = agents_dir.join(".deleted");
        let known_dir = agents_dir.join(".known");
        let known_index_file = agents_dir.join(".known_index");
        let known_index = store::load_known_index(&known_index_file, &known_dir, &root)?;
        for dir in [&sessions_dir, &deleted_dir] {
            std::fs::create_dir_all(dir).map_err(|e| AgentError::io_at(dir, e))?;
        }
        seed_instructions(&agents_dir)?;

        let mut sessions = store::load_sessions(&sessions_dir)?;
        let deleted = store::load_deleted(&deleted_dir)?;

        let reusable = sessions
            .iter()
            .next_back()
            .filter(|(_, turns)| turns.len() == 1 && turns[0].is_empty())
            .map(|(n, _)| *n);
        let session = match reusable {
            Some(n) => n,
            None => {
                let n = sessions.keys().next_back().map_or(1, |last| last + 1);
                let turns = vec![TurnRecord::new(TurnKey::new(n, 1))];
                store::save_session(&sessions_dir, n, &turns)?;
                sessions.insert(n, turns);
                n
            }
        };
        let active = sessions[&session]
            .last()
            .map_or(TurnKey::new(session, 1), |t| t.key);

        let mut project = Self {
            root,
            agents_dir,
            sessions_dir,
            deleted_dir,
            known_dir,
            known_index_file,
            known_index,
            sessions,
            deleted,
            active_baseline: Tree::new(),
            active_moves: BTreeMap::new(),
            active,
        };
        project.init_active_state()?;
        Ok(project)
    }

    pub(crate) fn save_known_index(&self) -> Result<()> {
        store::save_known_index(&self.known_index_file, &self.known_index)
    }

    /// Reconstruct `active_baseline` and `active_moves` when the active turn already has recorded
    /// file updates.
    pub(crate) fn init_active_state(&mut self) -> Result<()> {
        self.active_baseline.clear();
        self.active_moves.clear();
        let updates = self.active_turn()?.updates.clone();
        if updates.is_empty() {
            return Ok(());
        }
        let flat = self.flat_keys();
        let active_idx = flat.len().saturating_sub(1);
        for path in self.touched_paths(self.active) {
            let before = self.content_before(&flat, active_idx, &path)?;
            self.active_baseline.insert(path, before);
        }
        for op in &updates {
            if let FileOp::Move { path, to_path } = op {
                self.active_moves.insert(to_path.clone(), path.clone());
            }
        }
        Ok(())
    }

    // ----- lookups ---------------------------------------------------------------------

    /// All turns in chronological order.
    pub(crate) fn flat_keys(&self) -> Vec<TurnKey> {
        self.sessions
            .values()
            .flat_map(|t| t.iter().map(|r| r.key))
            .collect()
    }

    pub(crate) fn turn(&self, key: TurnKey) -> Option<&TurnRecord> {
        self.sessions
            .get(&key.session)?
            .iter()
            .find(|t| t.key == key)
    }

    pub(crate) fn active_turn(&self) -> Result<&TurnRecord> {
        self.turn(self.active)
            .ok_or_else(|| AgentError::NotFound(format!("active turn {}", self.active)))
    }

    pub(crate) fn active_turn_mut(&mut self) -> Result<&mut TurnRecord> {
        let key = self.active;
        self.sessions
            .get_mut(&key.session)
            .and_then(|turns| turns.iter_mut().find(|t| t.key == key))
            .ok_or_else(|| AgentError::NotFound(format!("active turn {key}")))
    }

    pub(crate) fn save_session(&self, session: u32) -> Result<()> {
        let turns = self
            .sessions
            .get(&session)
            .ok_or_else(|| AgentError::NotFound(format!("session {session:02}")))?;
        store::save_session(&self.sessions_dir, session, turns)
    }

    // ----- history reconstruction ------------------------------------------------------

    /// Paths a turn changed or deleted.
    pub(crate) fn touched_paths(&self, key: TurnKey) -> Vec<String> {
        let mut paths = BTreeSet::new();
        if let Some(turn) = self.turn(key) {
            for op in &turn.updates {
                for p in op.paths() {
                    paths.insert(p.to_string());
                }
            }
        }
        if let Some(del) = self.deleted.get(&key) {
            paths.extend(del.keys().cloned());
        }
        paths.into_iter().collect()
    }

    /// Content of `path` just before the turn at `flat[idx]` ran, i.e. after turn `idx - 1`.
    /// `idx == flat.len()` gives the current state on disk.
    ///
    /// Reconstructs the state by following `path` forward through `flat[idx..]` (tracking moves
    /// and patches until a deletion or the current disk state is reached) and then applying
    /// inverse hunks in reverse chronological order.
    pub(crate) fn content_before(
        &self,
        flat: &[TurnKey],
        idx: usize,
        path: &str,
    ) -> Result<Option<String>> {
        let mut cur_path = path.to_string();
        let mut patches_to_undo: Vec<(String, Vec<PatchHunk>)> = Vec::new();
        let mut terminal_text: Option<Option<String>> = None;

        for key in &flat[idx..] {
            let Some(turn) = self.turn(*key) else {
                continue;
            };
            let updates = &turn.updates;

            if updates
                .iter()
                .any(|op| matches!(op, FileOp::Delete { path: d } if d == &cur_path))
            {
                let content = self
                    .deleted
                    .get(key)
                    .and_then(|m| m.get(&cur_path))
                    .cloned()
                    .ok_or_else(|| {
                        AgentError::NotFound(format!(
                            "deleted file record for '{cur_path}' in turn {key}"
                        ))
                    })?;
                terminal_text = Some(Some(content));
                break;
            }

            if let Some(to_path) = updates.iter().find_map(|op| match op {
                FileOp::Move {
                    path: from,
                    to_path,
                } if from == &cur_path => Some(to_path.clone()),
                _ => None,
            }) {
                for op in updates {
                    if let FileOp::Patch { path: p, hunks } = op
                        && p == &to_path
                    {
                        patches_to_undo.push((to_path.clone(), hunks.clone()));
                    }
                }
                cur_path = to_path;
                continue;
            }

            let created_or_moved_into = updates.iter().any(|op| match op {
                FileOp::Create { path: c, .. } => c == &cur_path,
                FileOp::Move { to_path: to, .. } => to == &cur_path,
                _ => false,
            });
            if created_or_moved_into {
                terminal_text = Some(None);
                break;
            }

            for op in updates {
                if let FileOp::Patch { path: p, hunks } = op
                    && p == &cur_path
                {
                    patches_to_undo.push((cur_path.clone(), hunks.clone()));
                }
            }
        }

        let mut text = match terminal_text {
            Some(found) => found,
            None => {
                if !self.active_baseline.contains_key(&cur_path)
                    && let Some(known) = store::read_known(&self.known_dir, &cur_path)?
                {
                    Some(known)
                } else {
                    read_text_opt(&self.root, &cur_path)?
                }
            }
        };

        for (patched_path, hunks) in patches_to_undo.iter().rev() {
            let after_text = text.ok_or_else(|| {
                AgentError::NotFound(format!(
                    "file '{cur_path}' is missing on disk, needed to reconstruct '{path}'"
                ))
            })?;
            text = Some(apply_hunks_reverse(patched_path, &after_text, hunks)?);
        }
        Ok(text)
    }

    /// Language tooling from `.AGENTS/config.toml`, read on every call so edits apply at once.
    pub fn language_config(&self) -> Result<LanguageConfig> {
        LanguageConfig::load(&self.agents_dir)
    }
}

impl Workspace for Project {
    fn root(&self) -> &Path {
        &self.root
    }

    fn last_turn_touching(&self, rel: &str) -> Option<String> {
        self.flat_keys()
            .into_iter()
            .rev()
            .find(|key| {
                self.turn(*key)
                    .is_some_and(|turn| turn.updates.iter().any(|op| op.touches_path(rel)))
            })
            .map(|key| key.to_string())
    }

    fn read_at(&self, id: &str, rel: &str) -> Result<Option<String>> {
        let flat = self.flat_keys();
        let idx = match self.parse_selector(&flat, id)? {
            Sel::Turns { hi, .. } => hi,
            Sel::Update(key, _) => flat
                .iter()
                .position(|k| *k == key)
                .ok_or_else(|| AgentError::NotFound(format!("turn {key}")))?,
            _ => {
                return Err(AgentError::invalid(
                    "at_implicit_id",
                    "expected a turn, session or update ID",
                ));
            }
        };
        self.content_before(&flat, idx + 1, rel)
    }
}

/// Create the two usage guides if missing. `INSTRUCTION.md` is never created.
fn seed_instructions(agents: &Path) -> Result<()> {
    seed_guide(agents, AgentUseMode::Continuous)?;
    seed_guide(agents, AgentUseMode::SessionBased)?;
    Ok(())
}

/// Content of the guide for `mode`, created from the default when missing.
pub(crate) fn seed_guide(agents: &Path, mode: AgentUseMode) -> Result<String> {
    read_or_seed(agents, text::guide_file(mode), &text::default_guide(mode))
}

/// Read an instruction file; create it from `default` when missing.
fn read_or_seed(dir: &Path, name: &str, default: &str) -> Result<String> {
    let path = dir.join(name);
    match std::fs::read_to_string(&path) {
        Ok(content) => return Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(AgentError::io_at(&path, e)),
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => file
            .write_all(default.as_bytes())
            .map_err(|e| AgentError::io_at(&path, e))?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(AgentError::io_at(&path, e)),
    }
    Ok(default.to_string())
}
