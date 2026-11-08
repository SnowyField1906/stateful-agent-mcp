//! `workflow_*` tools on [`State`].

use serde_json::Value;

use crate::common::Result;
use crate::State;
use crate::workflow::tools::{
    GetFilesDiffParams, GetFilesHistoryParams, GetItemsParams, GetTurnsParams, NewTurnParams,
    RegisterParams, SearchParams, WriteFilesParams, WriteNotesParams, WriteTasksParams,
    get_files_diff, get_files_history, get_notes, get_tasks, get_turns, new_turn_with_paths,
    register, search, write_files_with_paths, write_notes, write_tasks,
};

impl State {
    pub fn workflow_register(&self, p: RegisterParams) -> Result<Value> {
        let (project, info) = register::register(self.config(), p)?;
        let mut slot = self.slot();
        *slot = Some(project);
        self.reset_hints();
        Ok(info)
    }

    pub fn workflow_new_turn(&self, p: NewTurnParams) -> Result<Value> {
        self.with_project_mut(|project| {
            let (mut out, changed_paths) = new_turn_with_paths(project, p)?;
            let turn_id = out
                .get("turn_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            self.maybe_add_validation_hint(project, &turn_id, &changed_paths, &mut out);
            Ok(out)
        })
    }

    pub fn workflow_write_files(&self, p: WriteFilesParams) -> Result<Value> {
        self.with_project_mut(|project| {
            let (mut out, changed_paths) = write_files_with_paths(project, p)?;
            let turn_id = out
                .get("turn_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            self.maybe_add_validation_hint(project, &turn_id, &changed_paths, &mut out);
            Ok(out)
        })
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
        self.with_project(|project| get_turns::get_turns(project, self.config(), p))
    }

    pub fn workflow_search(&self, p: SearchParams) -> Result<Value> {
        self.with_project(|project| search::search(project, self.config(), p))
    }

    pub fn workflow_get_files_diff(&self, p: GetFilesDiffParams) -> Result<String> {
        self.with_project(|project| get_files_diff::get_files_diff(project, p))
    }

    pub fn workflow_get_files_history(&self, p: GetFilesHistoryParams) -> Result<Value> {
        self.with_project(|project| get_files_history::get_files_history(project, self.config(), p))
    }
}
