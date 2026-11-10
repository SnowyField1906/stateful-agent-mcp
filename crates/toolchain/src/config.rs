//! Per-project language tooling read from `.AGENTS/config.toml`.
//!
//! ```toml
//! [rust]
//! format = "rustfmt --edition 2024"
//! test   = "cargo test {args}"
//!
//! [languages.go]          # any other language
//! lint = "go vet ./..."
//! ```
//!
//! Empty strings count as unset. A command receives caller arguments only where it contains the
//! `{args}` placeholder.

use std::collections::BTreeMap;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::common::pathspec::AGENTS_DIR;
use crate::common::{AgentError, Result};

/// File name under `.AGENTS/`.
pub const CONFIG_FILE: &str = "config.toml";

/// Placeholder replaced by the caller's arguments.
pub const ARGS_PLACEHOLDER: &str = "{args}";

/// A project command the agent may run. Declaration order is the order shown to the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Lint,
    Build,
    Test,
    Install,
    Remove,
}

impl Action {
    pub const ALL: [Action; 5] = [
        Action::Lint,
        Action::Build,
        Action::Test,
        Action::Install,
        Action::Remove,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Action::Lint => "lint",
            Action::Build => "build",
            Action::Test => "test",
            Action::Install => "install",
            Action::Remove => "remove",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "lint" => Some(Action::Lint),
            "build" => Some(Action::Build),
            "test" => Some(Action::Test),
            "install" => Some(Action::Install),
            "remove" => Some(Action::Remove),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Action::parse(&s).ok_or_else(|| {
            serde::de::Error::custom("expected lint, build, test, install or remove")
        })
    }
}

impl From<&str> for Action {
    fn from(s: &str) -> Self {
        Action::parse(s).unwrap_or_else(|| panic!("invalid action: {s}"))
    }
}

impl From<String> for Action {
    fn from(s: String) -> Self {
        Action::from(s.as_str())
    }
}

/// Commands configured for one language.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LanguageTooling {
    /// Formatter run automatically by `write_files`. One that reads stdin and writes stdout also
    /// tells the ledger which changes made outside it are only formatting.
    pub format: Option<String>,
    actions: BTreeMap<Action, String>,
}

impl LanguageTooling {
    pub fn action(&self, action: Action) -> Option<&str> {
        self.actions.get(&action).map(String::as_str)
    }

    /// Whether the command takes caller arguments.
    pub fn accepts_args(command: &str) -> bool {
        command.contains(ARGS_PLACEHOLDER)
    }

    fn has_actions(&self) -> bool {
        !self.actions.is_empty()
    }
}

/// Parsed `.AGENTS/config.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LanguageConfig {
    languages: BTreeMap<String, LanguageTooling>,
}

impl LanguageConfig {
    /// Read the configuration of the project rooted at `root`.
    pub fn load_at(root: &Path) -> Result<Self> {
        Self::load(&root.join(AGENTS_DIR))
    }

    /// Read `<agents_dir>/config.toml`. A missing file gives an empty configuration. The file is
    /// read on every call so edits take effect without restarting the server.
    pub fn load(agents_dir: &Path) -> Result<Self> {
        let path = agents_dir.join(CONFIG_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|reason| AgentError::Corrupt {
                path: path.display().to_string(),
                reason,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(AgentError::io_at(&path, e)),
        }
    }

    /// Parse configuration text. Top-level tables are languages; tables under `[languages]` are
    /// languages too.
    pub fn parse(text: &str) -> std::result::Result<Self, String> {
        let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
        let mut languages = BTreeMap::new();
        for (key, value) in &table {
            if key == "languages" {
                let nested = value
                    .as_table()
                    .ok_or_else(|| "`languages` must be a table of languages".to_string())?;
                for (name, section) in nested {
                    languages.insert(name.clone(), parse_section(name, section)?);
                }
            } else {
                languages.insert(key.clone(), parse_section(key, value)?);
            }
        }
        Ok(Self { languages })
    }

    pub fn language(&self, name: &str) -> Option<&LanguageTooling> {
        self.languages.get(name)
    }

    /// Actions per language for `workflow_register`, e.g. `{"rust": ["build", "test {args}"]}`.
    /// `format` is left out because it runs automatically. Languages without actions are left
    /// out; the result is empty when nothing is configured.
    pub fn capabilities(&self) -> BTreeMap<String, Vec<String>> {
        self.languages
            .iter()
            .filter(|(_, tooling)| tooling.has_actions())
            .map(|(name, tooling)| {
                let entries = Action::ALL
                    .into_iter()
                    .filter_map(|action| {
                        let command = tooling.action(action)?;
                        Some(if LanguageTooling::accepts_args(command) {
                            format!("{} {ARGS_PLACEHOLDER}", action.name())
                        } else {
                            action.name().to_string()
                        })
                    })
                    .collect();
                (name.clone(), entries)
            })
            .collect()
    }

    /// `capabilities` as JSON, or `None` when empty.
    pub fn capabilities_json(&self) -> Option<Value> {
        let caps = self.capabilities();
        (!caps.is_empty()).then(|| json!(caps))
    }
}

fn parse_section(name: &str, value: &toml::Value) -> std::result::Result<LanguageTooling, String> {
    let table = value
        .as_table()
        .ok_or_else(|| format!("`{name}` must be a table of commands"))?;
    let mut tooling = LanguageTooling::default();
    for (key, value) in table {
        let command = value
            .as_str()
            .ok_or_else(|| format!("`{name}.{key}` must be a string"))?
            .trim();
        if key == "format" {
            tooling.format = (!command.is_empty()).then(|| command.to_string());
        } else if let Some(action) = Action::parse(key) {
            if !command.is_empty() {
                tooling.actions.insert(action, command.to_string());
            }
        } else {
            return Err(format!(
                "unknown key `{name}.{key}`; expected format, lint, build, test, install or remove"
            ));
        }
    }
    Ok(tooling)
}
