//! A loaded project: sessions, `.AGENTS/.deleted/` records, and reconstruction of past file
//! contents by reverse-applying turn updates.
//!
//! Layout under `<root>/.AGENTS/`:
//! - `INSTRUCTION*.md` — protocol texts, seeded once and then owned by the project
//! - `sessions/{SS}.md` — turns, notes, net file updates, handover tasks
//! - `.deleted/{SS.TT}/{path}` — pre-turn content of files deleted in turn `SS.TT`. Any past
//!   state is rebuilt from current disk files, turn updates, and `.deleted/`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::config::{AgentUseMode, ServerConfig};
use crate::error::{AgentError, Result};
use crate::files::Tree;
use crate::files::overlay::read_text_opt;
use crate::files::patch::apply_hunks_reverse;
use crate::files::summary::op_summary;
use crate::instructions as text;
use crate::lang::LanguageConfig;
use crate::params::{FileOp, HandoverTaskDto, PatchHunk};
use crate::session::model::{TurnKey, TurnRecord};
use crate::session::store::{self, DeletedMap};

use super::selector::Sel;

/// Sessions listed in the registration response (most recent).
const SESSIONS_SHOWN: usize = 10;

pub struct Project {
    pub(crate) root: PathBuf,
    pub(crate) agents_dir: PathBuf,
    pub(crate) sessions_dir: PathBuf,
    pub(crate) deleted_dir: PathBuf,
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
            return Err(AgentError::invalid("folder_path", "must be an absolute path"));
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
        let agents_dir = root.join(".AGENTS");
        let sessions_dir = agents_dir.join("sessions");
        let deleted_dir = agents_dir.join(".deleted");
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
            sessions,
            deleted,
            active_baseline: Tree::new(),
            active_moves: BTreeMap::new(),
            active,
        };
        project.init_active_state()?;
        Ok(project)
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
        self.sessions.values().flat_map(|t| t.iter().map(|r| r.key)).collect()
    }

    pub(crate) fn turn(&self, key: TurnKey) -> Option<&TurnRecord> {
        self.sessions.get(&key.session)?.iter().find(|t| t.key == key)
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
            None => read_text_opt(&self.root, &cur_path)?,
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

    /// Content of `path` at the end of the turn(s) selected by `id` (a turn, session, `HEAD~N`
    /// or update ID).
    pub(crate) fn read_at(&self, id: &str, path: &str) -> Result<Option<String>> {
        let flat = self.flat_keys();
        let idx = match self.parse_selector(&flat, id)? {
            Sel::Turns { hi, .. } => hi,
            Sel::Update(key, _) => flat
                .iter()
                .position(|k| *k == key)
                .ok_or_else(|| AgentError::NotFound(format!("turn {key}")))?,
            _ => return Err(AgentError::invalid("at_implicit_id", "expected a turn, session or update ID")),
        };
        self.content_before(&flat, idx + 1, path)
    }

    /// The most recent turn that changed `path`.
    pub(crate) fn last_turn_touching(&self, path: &str) -> Option<TurnKey> {
        self.flat_keys().into_iter().rev().find(|k| {
            self.turn(*k)
                .is_some_and(|t| t.updates.iter().any(|op| op.touches_path(path)))
        })
    }

    /// Language tooling from `.AGENTS/config.toml`, read on every call so edits apply at once.
    pub fn language_config(&self) -> Result<LanguageConfig> {
        LanguageConfig::load(&self.agents_dir)
    }

    // ----- registration response -------------------------------------------------------

    pub fn register_response(
        &self,
        config: &ServerConfig,
        show_instructions: bool,
        show_last_turn: bool,
    ) -> Result<Value> {
        let mut out = Map::new();
        out.insert("project_root".into(), json!(self.root.to_string_lossy()));
        out.insert("active_turn_id".into(), json!(self.active.to_string()));
        match self.language_config() {
            Ok(cfg) => {
                if let Some(caps) = cfg.capabilities_json() {
                    out.insert("language_capabilities".into(), caps);
                }
            }
            Err(e) => {
                out.insert("language_config_error".into(), json!(e.to_string()));
            }
        }
        if show_instructions {
            out.insert("instructions".into(), json!(self.instructions_text(config.use_mode)?));
        }
        if show_last_turn
            && let Some(turn) = self.last_closed_turn()
        {
            let mut last = turn_json(turn);
            if turn.tasks.is_empty()
                && let Some((from_key, tasks)) = self.latest_tasks_before(turn.key)
                && let Some(obj) = last.as_object_mut()
            {
                obj.insert("handover_tasks".into(), json!(tasks));
                obj.insert("handover_tasks_from_turn".into(), json!(from_key.to_string()));
            }
            out.insert("last_turn".into(), last);
        }
        let summaries = self.session_summaries();
        let skip = summaries.len().saturating_sub(SESSIONS_SHOWN);
        out.insert("sessions".into(), json!(&summaries[skip..]));
        if skip > 0 {
            out.insert("sessions_total".into(), json!(summaries.len()));
        }
        Ok(Value::Object(out))
    }

    /// The latest non-empty turn before the active one.
    fn last_closed_turn(&self) -> Option<&TurnRecord> {
        self.flat_keys()
            .into_iter()
            .rev()
            .filter(|k| *k != self.active)
            .filter_map(|k| self.turn(k))
            .find(|t| !t.is_empty())
    }

    /// The most recent non-empty handover task list from turns strictly before `before_key`.
    fn latest_tasks_before(&self, before_key: TurnKey) -> Option<(TurnKey, Vec<HandoverTaskDto>)> {
        self.flat_keys()
            .into_iter()
            .rev()
            .filter(|k| *k < before_key)
            .filter_map(|k| self.turn(k))
            .find(|t| !t.tasks.is_empty())
            .map(|t| (t.key, t.task_dtos()))
    }

    fn session_summaries(&self) -> Vec<String> {
        self.sessions
            .iter()
            .map(|(n, turns)| {
                let notes: usize = turns.iter().map(|t| t.notes.len()).sum();
                let tasks: usize = turns.iter().map(|t| t.tasks.len()).sum();
                let updates: usize = turns.iter().map(|t| t.updates.len()).sum();
                format!(
                    "{n:02}: {} turns, {notes} notes, {tasks} handover tasks, {updates} file updates",
                    turns.len()
                )
            })
            .collect()
    }

    /// The usage guide for the configured mode and, if the project has one, its own
    /// `INSTRUCTION.md`.
    fn instructions_text(&self, mode: AgentUseMode) -> Result<Value> {
        let guide = seed_guide(&self.agents_dir, mode)?;
        let mut out = Map::new();
        out.insert("guide".into(), json!(guide.trim_end()));
        let path = self.agents_dir.join(text::PROJECT_FILE);
        match std::fs::read_to_string(&path) {
            Ok(project) if !project.trim().is_empty() => {
                out.insert("project".into(), json!(project.trim_end()));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(AgentError::io_at(&path, e)),
        }
        Ok(Value::Object(out))
    }
}

/// A turn as shown to the agent. Updates are summaries; `get_files_diff` returns the content.
fn turn_json(turn: &TurnRecord) -> Value {
    let mut m = Map::new();
    m.insert("turn_id".into(), json!(turn.key.to_string()));
    if let Some(title) = &turn.title {
        m.insert("title".into(), json!(title));
    }
    if !turn.notes.is_empty() {
        m.insert("notes".into(), json!(turn.note_dtos()));
    }
    if !turn.updates.is_empty() {
        let updates: Vec<String> = turn
            .updates
            .iter()
            .map(op_summary)
            .collect();
        m.insert("updates".into(), json!(updates));
    }
    if !turn.tasks.is_empty() {
        m.insert("handover_tasks".into(), json!(turn.task_dtos()));
    }
    Value::Object(m)
}

/// Create the two usage guides if missing. `INSTRUCTION.md` is never created.
fn seed_instructions(agents: &Path) -> Result<()> {
    seed_guide(agents, AgentUseMode::Continuous)?;
    seed_guide(agents, AgentUseMode::SessionBased)?;
    Ok(())
}

/// Content of the guide for `mode`, created from the default when missing.
fn seed_guide(agents: &Path, mode: AgentUseMode) -> Result<String> {
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
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => file
            .write_all(default.as_bytes())
            .map_err(|e| AgentError::io_at(&path, e))?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(AgentError::io_at(&path, e)),
    }
    Ok(default.to_string())
}
