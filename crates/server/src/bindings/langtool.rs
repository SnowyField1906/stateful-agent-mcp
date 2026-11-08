//! `langtool_*` tools on [`State`].

use crate::common::{Result, Workspace};
use crate::langtool::tools::{
    FindReferencesParams, FindSymbolsParams, ReadFileEntry, ReadFileParams, RunActionParams,
    ViewCallgraphParams, find_references, find_symbols, read_file, run_action, view_callgraph,
};
use crate::{State, steering};

impl State {
    pub fn langtool_read_file(&self, p: ReadFileParams) -> Result<String> {
        let focused = p.files.iter().any(ReadFileEntry::is_focused);
        let (out, unfocused) =
            self.with_workspace(|workspace| read_file::read_file(workspace, p))?;
        let hint = self.tool_hints().after_read(focused, unfocused);
        Ok(steering::hinted(out, hint))
    }

    pub fn langtool_find_symbols(&self, p: FindSymbolsParams) -> Result<String> {
        let out = self
            .with_workspace(|workspace| find_symbols::find_symbols(workspace, self.config(), p))?;
        self.tool_hints().end_grep_streak();
        Ok(out)
    }

    pub fn langtool_find_references(&self, p: FindReferencesParams) -> Result<String> {
        let out = self.with_workspace(|workspace| {
            find_references::find_references(workspace, self.config(), p)
        })?;
        self.tool_hints().end_grep_streak();
        Ok(out)
    }

    pub fn langtool_view_callgraph(&self, p: ViewCallgraphParams) -> Result<String> {
        let out = self.with_workspace(|workspace| {
            view_callgraph::view_callgraph(workspace, self.config(), p)
        })?;
        self.tool_hints().end_grep_streak();
        Ok(out)
    }

    pub fn langtool_run_action(&self, p: RunActionParams) -> Result<String> {
        let language = p.language.clone();
        let action = p.action;
        self.with_project(|project| {
            let ran = run_action::run_action(project.root(), p)?;
            if let Some(hint) = self.validation_action_ran(
                &project.active_turn_id(),
                &language,
                action,
                ran.succeeded,
            ) {
                return Ok(format!("{}hint: {hint}\n", ran.text));
            }
            Ok(ran.text)
        })
    }
}
