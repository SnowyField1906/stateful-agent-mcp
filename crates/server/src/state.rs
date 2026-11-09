//! Shared server state: the configuration and the registered project.
//!
//! Tool methods are added to [`State`] by `bindings/`.

use std::sync::{Mutex, MutexGuard};

use serde_json::{Value, json};

use crate::common::{AgentError, Result, ServerConfig, Workspace};
use crate::hints::ValidationHints;
use crate::langtool::engine::lens::store::FactStore;
use crate::steering::ToolUseHints;
use crate::toolchain::Action;
use crate::workflow::Project;

/// Capacity of the in-memory fact store: how many files' parsed facts are kept.
const FACT_STORE_CAPACITY: usize = 512;

pub struct State {
    config: ServerConfig,
    project: Mutex<Option<Project>>,
    validation_hints: Mutex<ValidationHints>,
    tool_hints: Mutex<ToolUseHints>,
    fact_store: FactStore,
}

impl State {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            project: Mutex::new(None),
            validation_hints: Mutex::new(ValidationHints::default()),
            tool_hints: Mutex::new(ToolUseHints::default()),
            fact_store: FactStore::new(FACT_STORE_CAPACITY),
        }
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// The shared fact store used by the overview tools.
    pub(crate) fn fact_store(&self) -> &FactStore {
        &self.fact_store
    }

    /// The registered project slot. A poisoned lock still holds consistent data: every write is
    /// persisted before it is reported.
    pub(crate) fn slot(&self) -> MutexGuard<'_, Option<Project>> {
        self.project.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `op` on the registered project.
    pub(crate) fn with_project<T>(&self, op: impl FnOnce(&Project) -> Result<T>) -> Result<T> {
        let slot = self.slot();
        op(slot.as_ref().ok_or(AgentError::ProjectNotRegistered)?)
    }

    /// Run `op` on the registered project, for tools that change it.
    pub(crate) fn with_project_mut<T>(
        &self,
        op: impl FnOnce(&mut Project) -> Result<T>,
    ) -> Result<T> {
        let mut slot = self.slot();
        op(slot.as_mut().ok_or(AgentError::ProjectNotRegistered)?)
    }

    /// Run `op` with the registered project as a [`Workspace`], if any. Read-only tools accept
    /// absolute paths without one.
    pub(crate) fn with_workspace<T>(
        &self,
        op: impl FnOnce(Option<&dyn Workspace>) -> Result<T>,
    ) -> Result<T> {
        let slot = self.slot();
        op(slot.as_ref().map(|project| project as &dyn Workspace))
    }

    /// Forget what the hints have seen: a newly registered project starts a fresh session.
    pub(crate) fn reset_hints(&self) {
        self.validation_hints
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reset();
        self.tool_hints().reset();
    }

    /// The hints that follow tool use across the whole session.
    pub(crate) fn tool_hints(&self) -> MutexGuard<'_, ToolUseHints> {
        self.tool_hints.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Add non-blocking hints after a real change: the configured checks to run, and handover
    /// tasks to record once a turn has piled up edit batches.
    pub(crate) fn maybe_add_validation_hint(
        &self,
        project: &Project,
        turn_id: &str,
        paths: &[String],
        output: &mut Value,
    ) {
        if paths.is_empty() || output.get("changed").and_then(Value::as_bool) == Some(false) {
            return;
        }
        let hint = {
            let mut hints = self
                .validation_hints
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            [
                hints.after_code_edit(project, turn_id, paths),
                hints.after_edit_batch(project, turn_id),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
        };
        if hint.is_empty() {
            return;
        }
        if let Some(fields) = output.as_object_mut() {
            fields.insert("hint".to_string(), json!(hint));
        }
    }

    /// Mark a configured validation attempt while the project lock is still held.
    pub(crate) fn validation_action_ran(
        &self,
        turn_id: &str,
        language: &str,
        action: Action,
        succeeded: bool,
    ) -> Option<String> {
        self.validation_hints
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .action_ran(turn_id, language, action, succeeded)
    }
}
