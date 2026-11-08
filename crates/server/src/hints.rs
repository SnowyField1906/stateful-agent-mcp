//! Turn-scoped reminders: configured checks to run after source edits, and handover tasks to
//! record after several edit batches.
use std::collections::{BTreeMap, BTreeSet};

use crate::language::language_for_path;
use crate::toolchain::Action;
use crate::workflow::Project;

/// Edit batches in one turn after which a turn without handover tasks is reminded to record some.
const EDIT_BATCHES_BEFORE_CHECKPOINT: usize = 4;

#[derive(Default)]
pub(crate) struct ValidationHints {
    turn_id: Option<String>,
    languages: BTreeMap<String, LanguageProgress>,
    edit_batches: usize,
    checkpoint_hinted: bool,
}

#[derive(Default)]
struct LanguageProgress {
    pending: BTreeSet<Action>,
    hinted: bool,
    validated_since_edit: bool,
}

impl ValidationHints {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Return one compact reminder for source languages changed in this turn.
    ///
    /// The state is deliberately session-local: a new turn starts clean, successful checks leave
    /// the pending set, and the next edit re-arms configured checks.
    pub(crate) fn after_code_edit(
        &mut self,
        project: &Project,
        turn_id: &str,
        paths: &[String],
    ) -> Option<String> {
        self.select_turn(turn_id);
        let config = project.language_config().ok()?;
        let languages: BTreeSet<&str> = paths
            .iter()
            .filter_map(|path| language_for_path(path).map(|language| language.name))
            .collect();

        let mut suggestions = Vec::new();
        for language in languages {
            let Some(tooling) = config.language(language) else {
                continue;
            };
            let checks: BTreeSet<Action> = [Action::Lint, Action::Build, Action::Test]
                .into_iter()
                .filter(|action| tooling.action(*action).is_some())
                .collect();
            if checks.is_empty() {
                continue;
            }

            let progress = self.languages.entry(language.to_string()).or_default();
            if progress.pending.is_empty() || progress.validated_since_edit {
                progress.pending.clone_from(&checks);
                progress.hinted = false;
                progress.validated_since_edit = false;
            }
            if !progress.hinted {
                let actions = progress
                    .pending
                    .iter()
                    .map(|action| action.name())
                    .collect::<Vec<_>>()
                    .join(", ");
                suggestions.push(format!("{}: {actions}", title_case(language)));
                progress.hinted = true;
            }
        }

        (!suggestions.is_empty()).then(|| {
            format!(
                "Configured checks are available: {}. Run the relevant check with langtool_run_action before wrapping up this turn.",
                suggestions.join("; ")
            )
        })
    }

    /// Count one batch of file edits. Once a turn has `EDIT_BATCHES_BEFORE_CHECKPOINT` of them and
    /// no handover tasks, return one reminder to record the next steps; it is not repeated in the
    /// same turn.
    pub(crate) fn after_edit_batch(&mut self, project: &Project, turn_id: &str) -> Option<String> {
        self.select_turn(turn_id);
        self.edit_batches += 1;
        if self.checkpoint_hinted
            || self.edit_batches < EDIT_BATCHES_BEFORE_CHECKPOINT
            || project.handover_task_count().ok()? > 0
        {
            return None;
        }
        self.checkpoint_hinted = true;
        Some(format!(
            "This turn has {} edit batches and no handover tasks; record the next steps with workflow_write_tasks.",
            self.edit_batches
        ))
    }

    /// Record a validation attempt and return the remaining checks after a success.
    pub(crate) fn action_ran(
        &mut self,
        turn_id: &str,
        language: &str,
        action: Action,
        succeeded: bool,
    ) -> Option<String> {
        self.select_turn(turn_id);
        let Some(progress) = self.languages.get_mut(language.trim()) else {
            return None;
        };
        if !matches!(action, Action::Lint | Action::Build | Action::Test)
            || !progress.pending.contains(&action)
        {
            return None;
        }
        if succeeded {
            progress.pending.remove(&action);
        }
        progress.validated_since_edit = true;
        let remaining = progress
            .pending
            .iter()
            .map(|action| action.name())
            .collect::<Vec<_>>();
        (succeeded && !remaining.is_empty()).then(|| {
            format!(
                "Remaining configured checks for {}: {}. Run one with langtool_run_action.",
                title_case(language.trim()),
                remaining.join(", ")
            )
        })
    }

    fn select_turn(&mut self, turn_id: &str) {
        if self.turn_id.as_deref() != Some(turn_id) {
            *self = Self {
                turn_id: Some(turn_id.to_string()),
                ..Self::default()
            };
        }
    }
}

fn title_case(language: &str) -> String {
    let mut chars = language.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_uppercase().chain(chars).collect()
}
