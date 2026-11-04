//! Server state: the registered project and the operations on it.
//!
//! [`State`] holds one [`Project`] behind a mutex. Every tool is a synchronous method here;
//! the server runs them on a blocking thread.

mod actions;
mod handover_tasks;
mod items;
mod notes;
mod project;
mod query;
mod selector;
mod turns;
mod write;

use std::sync::{Mutex, PoisonError};

use serde_json::Value;

use crate::config::ServerConfig;
use crate::error::{AgentError, Result};
use crate::fs_tools;
use crate::params::{
    CodeCallsParams, CodeReferencesParams, CodeSymbolsParams, CreateNewTurnParams, DiffParams,
    FsFileInfoParams, FsFindParams, FsGrepParams, FsLsParams, FsReadFilesParams, HistoryParams,
    IdsParams, RegisterProjectParams, RunProjectActionParams, WriteFilesParams, WriteNotesParams,
    WriteTasksParams,
};

pub use project::Project;

pub struct State {
    config: ServerConfig,
    project: Mutex<Option<Project>>,
}

impl State {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            project: Mutex::new(None),
        }
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Run `f` on the registered project.
    fn with<T>(&self, f: impl FnOnce(&mut Project) -> Result<T>) -> Result<T> {
        let mut guard = self.project.lock().unwrap_or_else(PoisonError::into_inner);
        let project = guard.as_mut().ok_or(AgentError::ProjectNotRegistered)?;
        f(project)
    }

    /// Run `f` with the registered project, if there is one. For the tools that also accept
    /// `path_absolute`, which need no project.
    fn with_optional<T>(&self, f: impl FnOnce(Option<&Project>) -> Result<T>) -> Result<T> {
        let guard = self.project.lock().unwrap_or_else(PoisonError::into_inner);
        f(guard.as_ref())
    }

    /// Register a project (or reuse the already-open project for the same canonical root)
    /// and return its registration response.
    pub fn workflow_register(&self, p: RegisterProjectParams) -> Result<Value> {
        let canonical = Project::canonical_root(&p.folder_path)?;
        let mut guard = self.project.lock().unwrap_or_else(PoisonError::into_inner);
        let same_root = guard.as_ref().is_some_and(|pr| pr.root == canonical);
        if !same_root {
            *guard = Some(Project::open(&p.folder_path)?);
        }
        let project = guard.as_ref().ok_or(AgentError::ProjectNotRegistered)?;
        project.register_response(
            &self.config,
            p.show_instructions.unwrap_or(true),
            p.show_last_turn_info.unwrap_or(true),
        )
    }

    pub fn workflow_new_turn(&self, p: CreateNewTurnParams) -> Result<Value> {
        self.with(|pr| pr.create_new_turn(p))
    }

    pub fn workflow_write_files(&self, p: WriteFilesParams) -> Result<Value> {
        self.with(|pr| pr.write_files(p.ops))
    }

    /// Runs while the project lock is held, so other tools wait for the command to finish.
    pub fn langtool_run_action(&self, p: RunProjectActionParams) -> Result<String> {
        self.with(|pr| pr.run_project_action(p))
    }

    pub fn workflow_write_notes(&self, p: WriteNotesParams) -> Result<Value> {
        self.with(|pr| pr.write_notes(&p.ops))
    }

    pub fn workflow_write_task(&self, p: WriteTasksParams) -> Result<Value> {
        self.with(|pr| pr.write_tasks(&p.ops))
    }

    pub fn workflow_get_notes(&self, p: IdsParams) -> Result<Value> {
        self.with(|pr| pr.notes_by_selectors(&p.implicit_ids))
    }

    pub fn workflow_get_tasks(&self, p: IdsParams) -> Result<Value> {
        self.with(|pr| pr.tasks_by_selectors(&p.implicit_ids))
    }

    pub fn workflow_get_files_diff(&self, p: DiffParams) -> Result<String> {
        self.with(|pr| pr.files_diff(&p.implicit_id, p.implicit_paths.as_deref()))
    }

    pub fn workflow_get_files_history(&self, p: HistoryParams) -> Result<Value> {
        self.with(|pr| {
            pr.files_history(
                p.implicit_id.as_deref(),
                p.implicit_paths.as_deref(),
                self.config.default_limit,
            )
        })
    }

    pub fn filesystem_ls(&self, p: FsLsParams) -> Result<String> {
        self.with_optional(|pr| fs_tools::ls(pr, &self.config, p))
    }

    pub fn filesystem_find(&self, p: FsFindParams) -> Result<String> {
        self.with_optional(|pr| fs_tools::find(pr, &self.config, p))
    }

    pub fn langtool_read_file(&self, p: FsReadFilesParams) -> Result<String> {
        self.with_optional(|pr| fs_tools::read_files(pr, p))
    }

    pub fn filesystem_grep(&self, p: FsGrepParams) -> Result<String> {
        self.with_optional(|pr| fs_tools::grep(pr, &self.config, p))
    }

    pub fn langtool_find_symbols(&self, p: CodeSymbolsParams) -> Result<String> {
        self.with_optional(|pr| fs_tools::symbols(pr, &self.config, p))
    }

    pub fn langtool_find_references(&self, p: CodeReferencesParams) -> Result<String> {
        self.with_optional(|pr| fs_tools::code_references(pr, &self.config, p))
    }

    pub fn langtool_view_callgraph(&self, p: CodeCallsParams) -> Result<String> {
        self.with_optional(|pr| fs_tools::code_calls(pr, &self.config, p))
    }

    pub fn filesystem_file_metadata(&self, p: FsFileInfoParams) -> Result<String> {
        self.with_optional(|pr| fs_tools::file_info(pr, p))
    }
}
