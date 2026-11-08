//! `stateful-agent` binary: an MCP server over stdio.

use clap::{Parser, ValueEnum};
use rmcp::{ServiceExt, transport::stdio};

use stateful_agent::{AgentServer, AgentUseMode, ServerConfig};
use stateful_agent_workflow::tools::RegisterParams;

/// Usage guide that `workflow_register` serves.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Mode {
    /// Agents that run many tool calls in one go (default).
    Continuous,
    /// Short sessions that may end at any time.
    SessionBased,
}

impl From<Mode> for AgentUseMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Continuous => AgentUseMode::Continuous,
            Mode::SessionBased => AgentUseMode::SessionBased,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "stateful-agent",
    version,
    about = "MCP server for coding agents: workflow state and atomic file ops, gitignore-aware filesystem search, and Tree-sitter code intelligence"
)]
struct Cli {
    /// Project root to register at startup (agents normally call `workflow_register` themselves).
    project: Option<String>,

    /// Usage guide to serve: `continuous` for agents that run many tool calls in one go,
    /// `session-based` for short sessions that may end at any time.
    #[arg(long, value_enum, default_value_t = Mode::Continuous)]
    mode: Mode,

    /// Maximum size of one tool result, in bytes.
    #[arg(long, default_value_t = ServerConfig::default().max_output_bytes)]
    max_output_bytes: usize,

    /// Default page size for listings.
    #[arg(long, default_value_t = ServerConfig::default().default_limit)]
    default_limit: usize,

    /// Maximum directory traversal depth for recursive listings and searches.
    #[arg(long, default_value_t = ServerConfig::default().max_dir_depth)]
    max_dir_depth: usize,
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter =
        EnvFilter::try_from_env("STATEFUL_AGENT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing();

    let config = ServerConfig {
        use_mode: cli.mode.into(),
        max_output_bytes: cli.max_output_bytes,
        default_limit: cli.default_limit.max(1),
        max_dir_depth: cli.max_dir_depth.max(1),
    };
    let server = AgentServer::new(config);

    if let Some(folder_path) = cli.project {
        let params = RegisterParams {
            folder_path,
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        };
        match server.state().workflow_register(params) {
            Ok(info) => {
                tracing::info!(project = %info["project_root"], "registered startup project")
            }
            Err(err) => tracing::warn!(error = %err, "startup project registration failed"),
        }
    }

    tracing::info!("serving on stdio");
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
