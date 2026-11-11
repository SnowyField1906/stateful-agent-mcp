//! The langtool tools the way the tests call them: a configuration and, optionally, a project
//! rooted at a scratch directory. Without one only absolute paths resolve.

use stateful_agent_common::{AgentError, Result, ServerConfig, Workspace};
use stateful_agent_langtool::engine::lens::store::FactStore;
use stateful_agent_langtool::tools::{
    FindReferencesParams, FindSymbolsParams, ReadFileParams, RunActionParams,
    SummarizeCodebaseParams, SummarizeFeatureParams, ViewCallgraphParams, find_references,
    find_symbols, read_file, run_action, summarize_codebase, summarize_feature, view_callgraph,
};
use stateful_agent_testkit::{FixedWorkspace, TempDir};

pub struct Tools {
    workspace: Option<FixedWorkspace>,
    config: ServerConfig,
}

impl Tools {
    /// No registered project.
    pub fn new(config: ServerConfig) -> Self {
        Self {
            workspace: None,
            config,
        }
    }

    /// A project rooted at `dir`.
    pub fn in_project(dir: &TempDir, config: ServerConfig) -> Self {
        Self {
            workspace: Some(dir.workspace()),
            config,
        }
    }

    fn workspace(&self) -> Option<&dyn Workspace> {
        self.workspace
            .as_ref()
            .map(|workspace| workspace as &dyn Workspace)
    }

    pub fn langtool_read_file(&self, p: ReadFileParams) -> Result<String> {
        Ok(read_file::read_file(self.workspace(), p)?.0)
    }

    pub fn langtool_find_symbols(&self, p: FindSymbolsParams) -> Result<String> {
        find_symbols::find_symbols(self.workspace(), &self.config, p)
    }

    pub fn langtool_find_references(&self, p: FindReferencesParams) -> Result<String> {
        find_references::find_references(self.workspace(), &self.config, p)
    }

    pub fn langtool_view_callgraph(&self, p: ViewCallgraphParams) -> Result<String> {
        view_callgraph::view_callgraph(self.workspace(), &self.config, p)
    }

    pub fn langtool_summarize_codebase(&self, p: SummarizeCodebaseParams) -> Result<String> {
        summarize_codebase::summarize_codebase(
            self.workspace(),
            &self.config,
            &FactStore::new(64),
            p,
        )
    }

    pub fn langtool_summarize_feature(&self, p: SummarizeFeatureParams) -> Result<String> {
        summarize_feature::summarize_feature(self.workspace(), &self.config, &FactStore::new(64), p)
    }

    pub fn langtool_run_action(&self, p: RunActionParams) -> Result<String> {
        let workspace = self
            .workspace
            .as_ref()
            .ok_or(AgentError::ProjectNotRegistered)?;
        Ok(run_action::run_action(workspace.root(), p)?.text)
    }
}
