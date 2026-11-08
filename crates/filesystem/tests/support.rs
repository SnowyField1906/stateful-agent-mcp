//! The filesystem tools the way the tests call them: no registered project, so only absolute
//! paths resolve, and a configuration.

use stateful_agent_common::{Result, ServerConfig};
use stateful_agent_filesystem::tools::{
    FileMetadataParams, FindParams, GrepParams, LsParams, file_metadata, find, grep, ls,
};

pub struct Tools {
    config: ServerConfig,
}

impl Tools {
    pub fn new(config: ServerConfig) -> Self {
        Self { config }
    }

    pub fn filesystem_ls(&self, p: LsParams) -> Result<String> {
        ls::ls(None, &self.config, p)
    }

    pub fn filesystem_find(&self, p: FindParams) -> Result<String> {
        find::find(None, &self.config, p)
    }

    pub fn filesystem_grep(&self, p: GrepParams) -> Result<String> {
        grep::grep(None, &self.config, p)
    }

    pub fn filesystem_file_metadata(&self, p: FileMetadataParams) -> Result<String> {
        file_metadata::file_metadata(None, p)
    }
}
