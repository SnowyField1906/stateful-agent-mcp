//! MCP server: exposes [`State`] operations as tools.
//!
//! Each tool runs its synchronous state method on a blocking thread and converts the
//! result with [`crate::output`]. Tool descriptions live in [`descriptions`].

mod descriptions;

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, InitializeResult, ServerCapabilities};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};

use crate::config::ServerConfig;
use crate::error::{AgentError, Result};
use crate::instructions;
use crate::output::{json_result, text_result};
use crate::params::{
    CodeCallsParams, CodeReferencesParams, CodeSymbolsParams, CreateNewTurnParams, DiffParams,
    FsFileInfoParams, FsFindParams, FsGrepParams, FsLsParams, FsReadFilesParams, HistoryParams,
    IdsParams, RegisterProjectParams, RunProjectActionParams, WriteFilesParams, WriteNotesParams,
    WriteTasksParams,
};
use crate::state::State;

#[derive(Clone)]
pub struct CodingAgentServer {
    state: Arc<State>,
}

impl CodingAgentServer {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            state: Arc::new(State::new(config)),
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    fn max_bytes(&self) -> usize {
        self.state.config().max_output_bytes
    }

    /// Run a synchronous state operation on a blocking thread.
    async fn run<T: Send + 'static>(
        &self,
        op: impl FnOnce(&State) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || op(&state))
            .await
            .map_err(|err| AgentError::Io(format!("tool task failed: {err}")))?
    }
}

#[tool_router]
impl CodingAgentServer {
    #[tool(
        description = descriptions::WORKFLOW_REGISTER,
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn workflow_register(
        &self,
        Parameters(p): Parameters<RegisterProjectParams>,
    ) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_register(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::WORKFLOW_NEW_TURN,
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn workflow_new_turn(
        &self,
        Parameters(p): Parameters<CreateNewTurnParams>,
    ) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_new_turn(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::WORKFLOW_WRITE_FILES,
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn workflow_write_files(
        &self,
        Parameters(p): Parameters<WriteFilesParams>,
    ) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_write_files(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::LANGTOOL_RUN_ACTION,
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn langtool_run_action(
        &self,
        Parameters(p): Parameters<RunProjectActionParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.langtool_run_action(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::WORKFLOW_WRITE_NOTES,
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn workflow_write_notes(
        &self,
        Parameters(p): Parameters<WriteNotesParams>,
    ) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_write_notes(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::WORKFLOW_WRITE_TASK,
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn workflow_write_task(
        &self,
        Parameters(p): Parameters<WriteTasksParams>,
    ) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_write_task(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::WORKFLOW_GET_NOTES,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn workflow_get_notes(&self, Parameters(p): Parameters<IdsParams>) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_get_notes(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::WORKFLOW_GET_TASKS,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn workflow_get_tasks(&self, Parameters(p): Parameters<IdsParams>) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_get_tasks(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::WORKFLOW_GET_FILES_DIFF,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn workflow_get_files_diff(
        &self,
        Parameters(p): Parameters<DiffParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.workflow_get_files_diff(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::WORKFLOW_GET_FILES_HISTORY,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn workflow_get_files_history(
        &self,
        Parameters(p): Parameters<HistoryParams>,
    ) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_get_files_history(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::FILESYSTEM_LS,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn filesystem_ls(&self, Parameters(p): Parameters<FsLsParams>) -> CallToolResult {
        text_result(
            self.run(move |s| s.filesystem_ls(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::FILESYSTEM_FIND,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn filesystem_find(&self, Parameters(p): Parameters<FsFindParams>) -> CallToolResult {
        text_result(
            self.run(move |s| s.filesystem_find(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::LANGTOOL_READ_FILE,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn langtool_read_file(
        &self,
        Parameters(p): Parameters<FsReadFilesParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.langtool_read_file(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::FILESYSTEM_GREP,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn filesystem_grep(&self, Parameters(p): Parameters<FsGrepParams>) -> CallToolResult {
        text_result(
            self.run(move |s| s.filesystem_grep(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::LANGTOOL_FIND_SYMBOLS,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn langtool_find_symbols(
        &self,
        Parameters(p): Parameters<CodeSymbolsParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.langtool_find_symbols(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::LANGTOOL_FIND_REFERENCES,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn langtool_find_references(
        &self,
        Parameters(p): Parameters<CodeReferencesParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.langtool_find_references(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::LANGTOOL_VIEW_CALLGRAPH,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn langtool_view_callgraph(
        &self,
        Parameters(p): Parameters<CodeCallsParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.langtool_view_callgraph(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = descriptions::FILESYSTEM_FILE_METADATA,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn filesystem_file_metadata(
        &self,
        Parameters(p): Parameters<FsFileInfoParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.filesystem_file_metadata(p)).await,
            self.max_bytes(),
        )
    }
}

#[tool_handler]
impl ServerHandler for CodingAgentServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(instructions::SERVER_INSTRUCTIONS)
    }
}
