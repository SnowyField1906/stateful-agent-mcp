//! `filesystem_*` tools on [`State`].

use crate::common::Result;
use crate::filesystem::tools::{
    FileMetadataParams, FindParams, GrepParams, LsParams, file_metadata, find, grep, ls,
};
use crate::{State, steering};

impl State {
    pub fn filesystem_ls(&self, p: LsParams) -> Result<String> {
        let out = self.with_workspace(|workspace| ls::ls(workspace, self.config(), p))?;
        let hint = self.tool_hints().after_ls(true);
        Ok(steering::hinted(out, hint))
    }

    pub fn filesystem_find(&self, p: FindParams) -> Result<String> {
        self.with_workspace(|workspace| find::find(workspace, self.config(), p))
    }

    pub fn filesystem_grep(&self, p: GrepParams) -> Result<String> {
        let symbol_search = steering::is_symbol_search(&p);
        let out = self.with_workspace(|workspace| grep::grep(workspace, self.config(), p))?;
        let hint = self.tool_hints().after_grep(symbol_search);
        Ok(steering::hinted(out, hint))
    }

    pub fn filesystem_file_metadata(&self, p: FileMetadataParams) -> Result<String> {
        self.with_workspace(|workspace| file_metadata::file_metadata(workspace, p))
    }
}
