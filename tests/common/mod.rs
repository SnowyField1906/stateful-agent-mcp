//! Helpers shared by the integration tests. Each test binary uses a subset.
#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use agent_mcp::ServerConfig;
use agent_mcp::server::State;
use agent_mcp::workflow::tools::RegisterParams;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A scratch directory under the system temp dir, removed on drop.
pub struct TempProject {
    pub root: PathBuf,
}

impl TempProject {
    pub fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "agent-mcp-test-{label}-{}-{id}",
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

    /// A server state with this directory registered as the project.
    pub fn open(&self, config: ServerConfig) -> State {
        let state = State::new(config);
        state
            .workflow_register(RegisterParams {
                folder_path: self.path_str(),
                show_instructions: Some(false),
                show_last_turn_info: Some(false),
            })
            .unwrap();
        state
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
