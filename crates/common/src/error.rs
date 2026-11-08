//! Crate error type.

use std::path::Path;

/// Crate-wide result alias.
pub type Result<T, E = AgentError> = std::result::Result<T, E>;

/// Errors returned to the agent as tool errors.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("no project registered; call `workflow_register` first")]
    ProjectNotRegistered,

    #[error("not found: {0}")]
    NotFound(String),

    #[error("path not found: {0}")]
    PathNotFound(String),

    #[error("turn {0} is closed; only the active turn can be modified")]
    ClosedTurn(String),

    #[error("patch failed on '{path}': {reason}")]
    PatchConflict { path: String, reason: String },

    #[error("invalid argument `{arg}`: {reason}")]
    InvalidArgument { arg: String, reason: String },

    #[error("protected path: {0}")]
    ProtectedPath(String),

    #[error("corrupt state file '{path}': {reason}")]
    Corrupt { path: String, reason: String },

    #[error("i/o error: {0}")]
    Io(String),
}

impl AgentError {
    pub fn invalid(arg: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidArgument {
            arg: arg.into(),
            reason: reason.into(),
        }
    }

    pub fn io_at(path: &Path, err: std::io::Error) -> Self {
        Self::Io(format!("{}: {err}", path.display()))
    }
}

impl From<std::io::Error> for AgentError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}
