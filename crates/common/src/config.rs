//! Runtime configuration.

/// Which agent profile the server serves instructions for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentUseMode {
    /// The agent makes many tool calls in one run.
    Continuous,
    /// Sessions are short and may end at any time.
    SessionBased,
}

/// Server settings.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Instruction profile loaded by `workflow_register`.
    pub use_mode: AgentUseMode,
    /// Maximum size of one tool result, in bytes.
    pub max_output_bytes: usize,
    /// Default page size for listings.
    pub default_limit: usize,
    /// Maximum directory traversal depth for recursive listings and searches.
    pub max_dir_depth: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            use_mode: AgentUseMode::Continuous,
            max_output_bytes: 64 << 10,
            default_limit: 200,
            max_dir_depth: 24,
        }
    }
}
