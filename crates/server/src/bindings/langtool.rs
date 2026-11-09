//! `langtool_*` tools on [`State`].

use crate::common::{Result, Workspace};
use crate::langtool::tools::{
    FindReferencesParams, FindSymbolsParams, ReadFileEntry, ReadFileParams, RunActionParams,
    SummarizeCodebaseParams, SummarizeFeatureParams, SummarizeSymbolParams, ViewCallgraphParams,
    find_references, find_symbols, read_file, run_action, summarize_codebase, summarize_feature,
    summarize_symbol, view_callgraph,
};
use crate::{State, steering};

/// The names of a lookup as owned strings, for the steering hints.
fn owned(names: Vec<&str>) -> Vec<String> {
    names.into_iter().map(str::to_string).collect()
}

impl State {
    pub fn langtool_read_file(&self, p: ReadFileParams) -> Result<String> {
        let focused = p.files.iter().any(ReadFileEntry::is_focused);
        let (out, unfocused) =
            self.with_workspace(|workspace| read_file::read_file(workspace, p))?;
        let hint = self.tool_hints().after_read(focused, unfocused);
        Ok(steering::hinted(out, hint))
    }

    pub fn langtool_find_symbols(&self, p: FindSymbolsParams) -> Result<String> {
        let names = p
            .query
            .as_ref()
            .map(|q| owned(q.cleaned()))
            .unwrap_or_default();
        let out = self
            .with_workspace(|workspace| find_symbols::find_symbols(workspace, self.config(), p))?;
        self.tool_hints().end_grep_streak();
        self.tool_hints().end_ls_streak();
        let hint = self.tool_hints().after_symbol_lookup(&names);
        Ok(steering::hinted(out, hint))
    }

    pub fn langtool_find_references(&self, p: FindReferencesParams) -> Result<String> {
        let names = owned(p.symbol.cleaned());
        let out = self.with_workspace(|workspace| {
            find_references::find_references(workspace, self.config(), p)
        })?;
        self.tool_hints().end_grep_streak();
        self.tool_hints().end_ls_streak();
        let hint = self.tool_hints().after_symbol_lookup(&names);
        Ok(steering::hinted(out, hint))
    }

    pub fn langtool_summarize_codebase(&self, p: SummarizeCodebaseParams) -> Result<String> {
        let out = self.with_workspace(|workspace| {
            summarize_codebase::summarize_codebase(workspace, self.config(), self.fact_store(), p)
        })?;
        self.tool_hints().end_ls_streak();
        self.tool_hints().end_lookup_streak();
        Ok(out)
    }

    pub fn langtool_summarize_symbol(&self, p: SummarizeSymbolParams) -> Result<String> {
        let out = self.with_workspace(|workspace| {
            summarize_symbol::summarize_symbol(workspace, self.config(), self.fact_store(), p)
        })?;
        self.tool_hints().end_grep_streak();
        self.tool_hints().end_ls_streak();
        self.tool_hints().end_lookup_streak();
        Ok(out)
    }

    pub fn langtool_summarize_feature(&self, p: SummarizeFeatureParams) -> Result<String> {
        let out = self.with_workspace(|workspace| {
            summarize_feature::summarize_feature(workspace, self.config(), self.fact_store(), p)
        })?;
        self.tool_hints().end_grep_streak();
        self.tool_hints().end_ls_streak();
        self.tool_hints().end_lookup_streak();
        Ok(out)
    }

    pub fn langtool_view_callgraph(&self, p: ViewCallgraphParams) -> Result<String> {
        let names = owned(p.symbol.cleaned());
        let out = self.with_workspace(|workspace| {
            view_callgraph::view_callgraph(workspace, self.config(), p)
        })?;
        self.tool_hints().end_grep_streak();
        let hint = self.tool_hints().after_symbol_lookup(&names);
        Ok(steering::hinted(out, hint))
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
