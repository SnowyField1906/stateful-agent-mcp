//! Instruction texts.
//!
//! Files under `.AGENTS/`:
//! - `INSTRUCTION.md`: rules of the project itself. Written by the user; never generated.
//! - `INSTRUCTION_FOR_CONTINUOUS_AGENTS.md`, `INSTRUCTION_FOR_SESSION_BASED_AGENTS.md`: usage
//!   guides for this MCP. Generated when missing, then owned by the project and editable.

use crate::config::AgentUseMode;

/// Sent to the client at MCP initialization.
pub const SERVER_INSTRUCTIONS: &str = "\
Start with `workflow_register` — it returns the usage guide, the project's own instructions and the previous turn's handover. \
[IF NEEDED] Inspect the cross-session context further with `workflow_get_files_diff` and `workflow_get_files_history`. \
[IF NEEDED] Use `filesystem_*` with any absolute path to read outside a registered project. \
[IF NEEDED] Use supported code intelligence `langtool_*` for advanced and systematical analyze the codebase. \
Begin a turn with `workflow_new_turn` — pass a `title` together with the turn's first updates (notes, tasks or file changes). \
Use `workflow_write_notes` only for important insights or durable findings that later turn could not guess. \
Use `workflow_write_task` for small and short-term work left for the next turn; for unfinished tasks — explicitly use RollOver. \
Use `workflow_write_files` (Create, Move, Patch, Delete, Recover) to update files; recommended to split into drip changes than all-at-once. \
[IF NEEDED] Use supported language actions with `langtool_run_action` in case it's needed to sacrifice token for further verification.";

/// Project-owned instructions. Optional; never generated.
pub const PROJECT_FILE: &str = "INSTRUCTION.md";
pub const CONTINUOUS_FILE: &str = "INSTRUCTION_FOR_CONTINUOUS_AGENTS.md";
pub const SESSION_FILE: &str = "INSTRUCTION_FOR_SESSION_BASED_AGENTS.md";

/// File name of the guide for `mode`.
pub fn guide_file(mode: AgentUseMode) -> &'static str {
    match mode {
        AgentUseMode::Continuous => CONTINUOUS_FILE,
        AgentUseMode::SessionBased => SESSION_FILE,
    }
}

/// Default content of the guide for `mode`.
pub fn default_guide(mode: AgentUseMode) -> String {
    let mode_text = match mode {
        AgentUseMode::Continuous => CONTINUOUS_SECTION,
        AgentUseMode::SessionBased => SESSION_SECTION,
    };
    format!("{COMMON_GUIDE}\n{mode_text}")
}

const COMMON_GUIDE: &str = "\
# MCP usage guide

## Concepts

- **Session**: starts with `workflow_register`. **Turn**: one unit of work inside a session, ID `SS.TT` (for example `01.03`). Only the active turn is writable; closed turns are immutable.
- **Notes**: concise durable facts or decisions a later turn needs. Include the reason or implication. Never log progress, tool/build status, duplicates, or information already in file updates. Leave notes empty when nothing qualifies. When a note builds on an earlier one, refer to it with `ref_notes` instead of repeating it.
- **Handover tasks**: short-term work left for the next turn or session. They are not the project's task list. Mark tasks the user asked for with `user_explicit: true`. Carry an unfinished task forward with `RollOver` instead of copying its content.
- **File updates**: `workflow_write_files` takes `Create`, `Move`, `Patch`, `Delete` and `Recover`. Inputs use `{op, ...fields}`. Results list one summary line per operation (`Patch src/a.rs +3 -1`), and `workflow_get_files_diff` returns a unified diff.

## Working in a turn

1. Read the previous turn's notes and handover tasks from `workflow_register`.
2. Inspect with `filesystem_ls`, `filesystem_find`, `filesystem_grep`, `langtool_read_file`; use `langtool_find_symbols` to outline Rust and Python definitions with their line ranges, `langtool_read_file` with `fold_level` to see a file's structure with deep blocks folded, `langtool_find_references` to find where a name is defined and used, and `langtool_view_callgraph` to see which functions a function calls and which call it, before reading. Pass a `files` list with `{file_path, lines?, numbered?, fold_level?}` per file to read related files in one call; read only what the task needs. `lines` and the listing parameter `entries` take the same `START..END` range syntax (`10..50`, `10..`, `..50`, a single number, or `-30..` for the last 30), and each result prints the next range to ask for. Any absolute path is read directly without a registered project.
3. Give a turn its title when it begins: pass `title` to `workflow_new_turn` together with the turn's first notes, tasks or file changes (a title alone is rejected).
4. Put related edits in one `workflow_write_files` call by default. A call is atomic and recorded immediately, and repeated calls in one turn merge into one net diff. For very long files (patch section by section) or large multi-file changes, split across several calls as checkpoints.
5. Use `Create` only for new files and `Patch` for all edits to existing files. Each hunk's `old_text` must match once (or be disambiguated by `start_line`).
6. Record unfinished work in handover tasks.
7. Review with `workflow_get_files_diff` and `workflow_get_files_history`. Restore earlier content with `Recover`.

## Style

Write plain, precise English: facts, constraints, reasons. Avoid emphasis, hype and alarming wording.
";

const CONTINUOUS_SECTION: &str = "\
## Continuous mode

You can run many tool calls without pausing, so work in steps, the way you would while building a feature.

- A turn holds one coherent step. Once the step is settled, say what comes next in handover tasks; if the project is left broken or half-finished, describe that in a note.
- Handover tasks are what the next session reads first, so keep them current as you go.
- The next step begins in its own turn, with a `title` and its first change.
- Keep turns small, about what a session-based agent finishes in one session, rather than putting a whole task into one turn.
";

const SESSION_SECTION: &str = "\
## Session-based mode

The session may stop at any time; leave a clear handover for unfinished work.

- Keep one coherent unit of work in the active turn. Settle it by recording what is left in `workflow_write_task`; a different unit of work begins in its own turn.
- Do not start a new turn just because the session continues or the same unit has more work.
- Mark user-requested tasks `user_explicit: true` and roll unfinished tasks of earlier turns over with `RollOver`.
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guides_differ_by_mode() {
        let c = default_guide(AgentUseMode::Continuous);
        let s = default_guide(AgentUseMode::SessionBased);
        assert!(c.contains("Continuous mode") && !c.contains("Session-based mode"));
        assert!(s.contains("Session-based mode") && !s.contains("Continuous mode"));
        assert!(c.starts_with("# MCP usage guide"));
    }
}
