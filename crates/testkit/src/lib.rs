//! Helpers the tests of the layer crates share. Not a layer: only `[dev-dependencies]` list it.
//!
//! - [`TempDir`]: a scratch directory under the system temp dir, removed on drop.
//! - [`LIB`]: a small Rust library with a type, its methods and two free functions.
//! - [`FixedWorkspace`]: a [`Workspace`] with a root and a small fixed turn history, so the read
//!   tools of `filesystem` and `langtool` run without a registered project.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use stateful_agent_common::{Result, Workspace};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A small library with a type, its methods and two free functions.
pub const LIB: &str = concat!(
    "pub struct Store;\n",
    "impl Store {\n",
    "    pub fn new() -> Store { Store }\n",
    "    pub fn put(\n",
    "        &self,\n",
    "        key: &str,\n",
    "    ) -> Option<String> {\n",
    "        let v = Some(key.to_string());\n",
    "        v.map(|s| s)\n",
    "    }\n",
    "}\n",
    "pub fn open() -> Store { Store::new() }\n",
    "pub fn setup() { let s = open(); s.put(\"k\"); }\n",
);

/// A scratch directory under the system temp dir, removed on drop.
pub struct TempDir {
    pub root: PathBuf,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "stateful-agent-test-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub fn path_str(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    /// Writes `text` to `rel`, creating parent directories.
    pub fn write(&self, rel: &str, text: &str) {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A workspace rooted here, with no turn history.
    pub fn workspace(&self) -> FixedWorkspace {
        FixedWorkspace::new(self.root.clone())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A [`Workspace`] with a fixed root. Turn history is empty unless [`FixedWorkspace::touched`] and
/// [`FixedWorkspace::at`] add some.
pub struct FixedWorkspace {
    root: PathBuf,
    last_turns: HashMap<String, String>,
    history: HashMap<(String, String), Option<String>>,
}

impl FixedWorkspace {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            last_turns: HashMap::new(),
            history: HashMap::new(),
        }
    }

    /// `turn` (`SS.TT`) is the last turn that changed `rel`.
    pub fn touched(mut self, rel: &str, turn: &str) -> Self {
        self.last_turns.insert(rel.to_string(), turn.to_string());
        self
    }

    /// `rel` has `content` at the end of `id` (`None`: the file did not exist then).
    pub fn at(mut self, id: &str, rel: &str, content: Option<&str>) -> Self {
        self.history.insert(
            (id.to_string(), rel.to_string()),
            content.map(str::to_string),
        );
        self
    }
}

impl Workspace for FixedWorkspace {
    fn root(&self) -> &Path {
        &self.root
    }

    fn last_turn_touching(&self, rel: &str) -> Option<String> {
        self.last_turns.get(rel).cloned()
    }

    fn read_at(&self, id: &str, rel: &str) -> Result<Option<String>> {
        Ok(self
            .history
            .get(&(id.to_string(), rel.to_string()))
            .cloned()
            .flatten())
    }
}
