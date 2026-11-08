//! A scratch project directory that opens as the registered project of a [`State`].

use std::ops::Deref;

use stateful_agent_common::ServerConfig;
use stateful_agent_server::State;
use stateful_agent_testkit::TempDir;
use stateful_agent_workflow::tools::RegisterParams;

pub struct TempProject {
    dir: TempDir,
}

impl TempProject {
    pub fn new(label: &str) -> Self {
        Self {
            dir: TempDir::new(label),
        }
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

impl Deref for TempProject {
    type Target = TempDir;

    fn deref(&self) -> &TempDir {
        &self.dir
    }
}
