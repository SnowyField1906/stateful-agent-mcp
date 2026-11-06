//! `langtool_run_action`: runs a configured `lint`, `build`, `test`, `install` or `remove` command.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::common::{AgentError, Result};
use crate::toolchain::Action;
use crate::toolchain::actions::{ACTION_TIMEOUT, plan, report};
use crate::toolchain::exec::run_shell;
use crate::workflow::Project;

pub const DESCRIPTION: &str = "Run a command the user configured for `language` and `action` (`lint`, `build`, `test`, `install`, `remove`); `workflow_register` lists them under `language_capabilities`. An action listed as `test {args}` takes `args` (`--help` shows what it accepts, an empty string passes none). Returns the command, exit status, stdout and stderr; 10-minute limit. Other tools wait while it runs.";

/// Run a command configured in `.AGENTS/config.toml`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunActionParams {
    /// Language section of the configuration, for example `rust` or `python`.
    pub language: String,
    /// One of `lint`, `build`, `test`, `install`, `remove`.
    pub action: String,
    /// Arguments, for commands configured with `{args}`. Pass an empty string to run such a
    /// command without arguments, or `--help` to see what it accepts.
    #[serde(default)]
    pub args: Option<String>,
}

pub fn run_action(project: &Project, p: RunActionParams) -> Result<String> {
    run_action_with_status(project, p).map(|(text, _)| text)
}

pub(crate) fn run_action_with_status(
    project: &Project,
    p: RunActionParams,
) -> Result<(String, bool)> {
    let language = p.language.trim();
    let action = Action::parse(p.action.trim()).ok_or_else(|| {
        AgentError::invalid("action", "expected lint, build, test, install or remove")
    })?;
    let config = project.language_config()?;
    let tooling = config.language(language).ok_or_else(|| {
        AgentError::NotFound(format!(
            "language `{language}` is not configured in .AGENTS/config.toml"
        ))
    })?;
    let command = tooling.action(action).ok_or_else(|| {
        AgentError::NotFound(format!(
            "action `{}` is not configured for `{language}` in .AGENTS/config.toml",
            action.name()
        ))
    })?;
    let planned = plan(language, action, command, p.args.as_deref());
    let out = run_shell(&planned.command, &project.root, None, ACTION_TIMEOUT)?;
    let succeeded = out.success();
    let mut text = report(&planned.command, &out);
    if let Some(hint) = planned.hint {
        text.push_str(&hint);
        text.push('\n');
    }
    Ok((text, succeeded))
}
