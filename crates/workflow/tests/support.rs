//! The workflow tools the way the tests call them: a `Session` over a [`Project`] with the method
//! names of the MCP tools, and a scratch directory that opens as a project. The server's hints are
//! not added.

use std::ops::Deref;
use std::sync::{Mutex, MutexGuard};

use serde_json::Value;
use stateful_agent_common::{AgentError, Result, ServerConfig, Workspace};
use stateful_agent_langtool::tools::{ReadFileParams, read_file};
use stateful_agent_testkit::TempDir;
use stateful_agent_workflow::Project;
use stateful_agent_workflow::tools::{
    GetFilesDiffParams, GetFilesHistoryParams, GetItemsParams, GetTurnsParams, NewTurnParams,
    RegisterParams, SearchParams, WriteFilesParams, WriteNotesParams, WriteTasksParams,
    get_files_diff, get_files_history, get_notes, get_tasks, get_turns, new_turn, register, search,
    write_files, write_notes, write_tasks,
};

pub struct Session {
    config: ServerConfig,
    project: Mutex<Option<Project>>,
}

impl Session {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            project: Mutex::new(None),
        }
    }

    fn slot(&self) -> MutexGuard<'_, Option<Project>> {
        self.project.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn with_project<T>(&self, op: impl FnOnce(&Project) -> Result<T>) -> Result<T> {
        let slot = self.slot();
        op(slot.as_ref().ok_or(AgentError::ProjectNotRegistered)?)
    }

    fn with_project_mut<T>(&self, op: impl FnOnce(&mut Project) -> Result<T>) -> Result<T> {
        let mut slot = self.slot();
        op(slot.as_mut().ok_or(AgentError::ProjectNotRegistered)?)
    }

    pub fn workflow_register(&self, p: RegisterParams) -> Result<Value> {
        let (project, info) = register::register(&self.config, p)?;
        *self.slot() = Some(project);
        Ok(info)
    }

    pub fn workflow_new_turn(&self, p: NewTurnParams) -> Result<Value> {
        self.with_project_mut(|project| new_turn::new_turn(project, p))
    }

    pub fn workflow_write_files(&self, p: WriteFilesParams) -> Result<Value> {
        self.with_project_mut(|project| write_files::write_files(project, p))
    }

    pub fn workflow_write_notes(&self, p: WriteNotesParams) -> Result<Value> {
        self.with_project_mut(|project| write_notes::write_notes(project, p))
    }

    pub fn workflow_write_tasks(&self, p: WriteTasksParams) -> Result<Value> {
        self.with_project_mut(|project| write_tasks::write_tasks(project, p))
    }

    pub fn workflow_get_notes(&self, p: GetItemsParams) -> Result<Value> {
        self.with_project(|project| get_notes::get_notes(project, p))
    }

    pub fn workflow_get_tasks(&self, p: GetItemsParams) -> Result<Value> {
        self.with_project(|project| get_tasks::get_tasks(project, p))
    }

    pub fn workflow_get_turns(&self, p: GetTurnsParams) -> Result<Value> {
        self.with_project(|project| get_turns::get_turns(project, &self.config, p))
    }

    pub fn workflow_search(&self, p: SearchParams) -> Result<Value> {
        self.with_project(|project| search::search(project, &self.config, p))
    }

    pub fn workflow_get_files_diff(&self, p: GetFilesDiffParams) -> Result<String> {
        self.with_project(|project| get_files_diff::get_files_diff(project, p))
    }

    pub fn workflow_get_files_history(&self, p: GetFilesHistoryParams) -> Result<Value> {
        self.with_project(|project| get_files_history::get_files_history(project, &self.config, p))
    }

    pub fn langtool_read_file(&self, p: ReadFileParams) -> Result<String> {
        self.with_project(|project| {
            read_file::read_file(Some(project as &dyn Workspace), p).map(|(out, _)| out)
        })
    }
}

/// A scratch project directory (a [`TempDir`]) that opens as a project of a [`Session`].
pub struct TempProject {
    dir: TempDir,
}

impl TempProject {
    pub fn new(label: &str) -> Self {
        Self {
            dir: TempDir::new(label),
        }
    }

    /// A session with this directory registered as the project.
    pub fn open(&self, config: ServerConfig) -> Session {
        let session = Session::new(config);
        session
            .workflow_register(RegisterParams {
                folder_path: self.path_str(),
                show_instructions: Some(false),
                show_last_turn_info: Some(false),
            })
            .unwrap();
        session
    }
}

impl Deref for TempProject {
    type Target = TempDir;

    fn deref(&self) -> &TempDir {
        &self.dir
    }
}
