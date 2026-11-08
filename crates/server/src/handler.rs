//! MCP handler: exposes [`State`] operations as tools.
//!
//! Each tool runs its synchronous state method on a blocking thread and converts the
//! result with [`super::output`]. Each tool's description is the `DESCRIPTION` of its tool file.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, InitializeResult, ServerCapabilities};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};

use super::State;
use super::output::{json_result, text_result};
use crate::common::{AgentError, Result, ServerConfig};
use crate::filesystem::tools::{
    self as filesystem, FileMetadataParams, FindParams, GrepParams, LsParams,
};
use crate::langtool::tools::{
    self as langtool, FindReferencesParams, FindSymbolsParams, ReadFileParams, RunActionParams,
    ViewCallgraphParams,
};
use crate::workflow::tools::{
    self as workflow, GetFilesDiffParams, GetFilesHistoryParams, GetItemsParams, GetTurnsParams,
    NewTurnParams, RegisterParams, SearchParams, WriteFilesParams, WriteNotesParams,
    WriteTasksParams,
};

/// Sent to the client at MCP initialization.
const INSTRUCTIONS: &str = "\
Call `workflow_register` first: it returns the usage guide, the project's instructions and the previous turn's handover. \
The read tools (`filesystem_*`, `langtool_read_file`, `langtool_find_*`, `langtool_view_callgraph`) also take absolute paths and then need no registered project. \
Explore code with `langtool_find_symbols`, `langtool_read_file` (`fold_level` for an outline, `symbol` for one definition), `langtool_find_references` and `langtool_view_callgraph` before reading whole files. \
Find earlier work with `workflow_search`, `workflow_get_turns`, `workflow_get_files_diff` and `workflow_get_files_history`. \
Open each unit of work with `workflow_new_turn` (a title and the first change). \
Change files only with `workflow_write_files`. \
Record durable decisions in `workflow_write_notes` and unfinished work in `workflow_write_tasks` (carry tasks over with RollOver). \
Verify with `langtool_run_action` when the project configures lint, build or test.";

#[derive(Clone)]
pub struct AgentServer {
    state: Arc<State>,
}

impl AgentServer {
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
impl AgentServer {
    #[tool(
        description = workflow::register::DESCRIPTION,
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn workflow_register(&self, Parameters(p): Parameters<RegisterParams>) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_register(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = workflow::new_turn::DESCRIPTION,
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn workflow_new_turn(&self, Parameters(p): Parameters<NewTurnParams>) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_new_turn(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = workflow::write_files::DESCRIPTION,
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
        description = langtool::run_action::DESCRIPTION,
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn langtool_run_action(
        &self,
        Parameters(p): Parameters<RunActionParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.langtool_run_action(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = workflow::write_notes::DESCRIPTION,
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
        description = workflow::write_tasks::DESCRIPTION,
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn workflow_write_tasks(
        &self,
        Parameters(p): Parameters<WriteTasksParams>,
    ) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_write_tasks(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = workflow::get_notes::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn workflow_get_notes(
        &self,
        Parameters(p): Parameters<GetItemsParams>,
    ) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_get_notes(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = workflow::get_tasks::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn workflow_get_tasks(
        &self,
        Parameters(p): Parameters<GetItemsParams>,
    ) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_get_tasks(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = workflow::get_turns::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn workflow_get_turns(
        &self,
        Parameters(p): Parameters<GetTurnsParams>,
    ) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_get_turns(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = workflow::search::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn workflow_search(&self, Parameters(p): Parameters<SearchParams>) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_search(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = workflow::get_files_diff::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn workflow_get_files_diff(
        &self,
        Parameters(p): Parameters<GetFilesDiffParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.workflow_get_files_diff(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = workflow::get_files_history::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn workflow_get_files_history(
        &self,
        Parameters(p): Parameters<GetFilesHistoryParams>,
    ) -> CallToolResult {
        json_result(
            self.run(move |s| s.workflow_get_files_history(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = filesystem::ls::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn filesystem_ls(&self, Parameters(p): Parameters<LsParams>) -> CallToolResult {
        text_result(
            self.run(move |s| s.filesystem_ls(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = filesystem::find::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn filesystem_find(&self, Parameters(p): Parameters<FindParams>) -> CallToolResult {
        text_result(
            self.run(move |s| s.filesystem_find(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = langtool::read_file::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn langtool_read_file(
        &self,
        Parameters(p): Parameters<ReadFileParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.langtool_read_file(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = filesystem::grep::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn filesystem_grep(&self, Parameters(p): Parameters<GrepParams>) -> CallToolResult {
        text_result(
            self.run(move |s| s.filesystem_grep(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = langtool::find_symbols::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn langtool_find_symbols(
        &self,
        Parameters(p): Parameters<FindSymbolsParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.langtool_find_symbols(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = langtool::find_references::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn langtool_find_references(
        &self,
        Parameters(p): Parameters<FindReferencesParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.langtool_find_references(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = langtool::view_callgraph::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn langtool_view_callgraph(
        &self,
        Parameters(p): Parameters<ViewCallgraphParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.langtool_view_callgraph(p)).await,
            self.max_bytes(),
        )
    }

    #[tool(
        description = filesystem::file_metadata::DESCRIPTION,
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn filesystem_file_metadata(
        &self,
        Parameters(p): Parameters<FileMetadataParams>,
    ) -> CallToolResult {
        text_result(
            self.run(move |s| s.filesystem_file_metadata(p)).await,
            self.max_bytes(),
        )
    }
}

#[tool_handler]
impl ServerHandler for AgentServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(INSTRUCTIONS)
    }
}
