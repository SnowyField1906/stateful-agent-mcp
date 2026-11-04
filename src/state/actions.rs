//! `langtool_run_action`: runs a configured `lint`, `build`, `test`, `install` or `remove` command.

use crate::error::{AgentError, Result};
use crate::lang::Action;
use crate::lang::actions::{ACTION_TIMEOUT, plan, report};
use crate::lang::exec::run_shell;
use crate::params::RunProjectActionParams;

use super::project::Project;

impl Project {
    pub fn run_project_action(&self, p: RunProjectActionParams) -> Result<String> {
        let language = p.language.trim();
        let action = Action::parse(p.action.trim()).ok_or_else(|| {
            AgentError::invalid("action", "expected lint, build, test, install or remove")
        })?;
        let config = self.language_config()?;
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
        let out = run_shell(&planned.command, &self.root, None, ACTION_TIMEOUT)?;
        let mut text = report(&planned.command, &out);
        if let Some(hint) = planned.hint {
            text.push_str(&hint);
            text.push('\n');
        }
        Ok(text)
    }
}
