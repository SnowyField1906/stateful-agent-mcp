//! `langtool_*` tools on [`State`].

use crate::common::Result;
use crate::langtool::tools::{
    FindReferencesParams, FindSymbolsParams, ReadFileEntry, ReadFileParams, RunActionParams,
    ViewCallgraphParams, find_references, find_symbols, read_file, run_action, view_callgraph,
};
use crate::server::{State, steering};

impl State {
    pub fn langtool_read_file(&self, p: ReadFileParams) -> Result<String> {
        let focused = p.files.iter().any(ReadFileEntry::is_focused);
        let (out, unfocused) =
            self.with_optional_project(|project| read_file::read_file(project, p))?;
        let hint = self.tool_hints().after_read(focused, unfocused);
        Ok(steering::hinted(out, hint))
    }

    pub fn langtool_find_symbols(&self, p: FindSymbolsParams) -> Result<String> {
        let out = self.with_optional_project(|project| {
            find_symbols::find_symbols(project, self.config(), p)
        })?;
        self.tool_hints().end_grep_streak();
        Ok(out)
    }

    pub fn langtool_find_references(&self, p: FindReferencesParams) -> Result<String> {
        let out = self.with_optional_project(|project| {
            find_references::find_references(project, self.config(), p)
        })?;
        self.tool_hints().end_grep_streak();
        Ok(out)
    }

    pub fn langtool_view_callgraph(&self, p: ViewCallgraphParams) -> Result<String> {
        let out = self.with_optional_project(|project| {
            view_callgraph::view_callgraph(project, self.config(), p)
        })?;
        self.tool_hints().end_grep_streak();
        Ok(out)
    }

    pub fn langtool_run_action(&self, p: RunActionParams) -> Result<String> {
        let language = p.language.clone();
        let action = p.action;
        self.with_project(|project| {
            let (out, succeeded) = run_action::run_action_with_status(project, p)?;
            if let Some(hint) = self.validation_action_ran(
                &project.active.to_string(),
                &language,
                action,
                succeeded,
            ) {
                return Ok(format!("{out}hint: {hint}\n"));
            }
            Ok(out)
        })
    }
}
