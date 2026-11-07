//! Instruction texts.
//!
//! Files under `.AGENTS/`:
//! - `INSTRUCTION.md`: rules of the project itself. Written by the user; never generated.
//! - `INSTRUCTION_FOR_CONTINUOUS_AGENTS.md`, `INSTRUCTION_FOR_SESSION_BASED_AGENTS.md`: usage
//!   guides for this MCP. Generated when missing, then owned by the project and editable.

use crate::common::AgentUseMode;

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

- **Session**: starts with `workflow_register`. **Turn**: one unit of work in a session, ID `SS.TT` (for example `01.03`). Only the active turn is writable; closed turns are immutable.
- **Notes**: durable facts or decisions that a later turn cannot guess, with the reason or implication. Never progress, build status, duplicates or what the file updates already show; leave notes empty when nothing qualifies. Link related notes with `ref_notes`.
- **Handover tasks**: short-term work left for the next turn or session, not the project's backlog. Mark tasks the user asked for `user_explicit: true`; carry unfinished ones forward with `RollOver`.
- **File updates**: `workflow_write_files` takes `Create`, `Move`, `Patch`, `Delete` and `Recover`. Results list one line per operation (`Patch src/a.rs +3 -1`); `workflow_get_files_diff` returns a unified diff.
- **External edits**: when a file the ledger has touched was changed on disk by someone else, the next `workflow_register`, `workflow_write_files` or `workflow_new_turn` records the change as its own turn titled `[External disk edit]` and lists it under `external_edits`. Later turns start from the edited content, so their diffs hold only your changes.

## Working in a turn

1. Read the previous handover from `workflow_register`. Before touching an area, run `workflow_search` for earlier decisions about it; `workflow_get_turns` shows the timeline.
2. Explore before reading whole files: `langtool_find_symbols` (definitions with line ranges; `docs` adds the first doc line, `include_body` prints definition bodies; batch queries supported), `langtool_read_file` with `fold_level: 0` (outline of a file, methods included; also for Markdown, TOML and other text) or with `symbol` (definitions by name in source order), `langtool_find_references` (definitions, calls, imports for `symbol`, or outgoing dependencies with `from_symbol`; batch supported) and `langtool_view_callgraph` (callers and callees; batch supported). Use `filesystem_ls`, `filesystem_find` and `filesystem_grep` for everything else. Read related files in one `langtool_read_file` call. `lines` and `entries` take `START..END` (`10..50`, `10..`, `..50`, `-30..`), and a cut result prints the next range. Absolute paths are read without a registered project.
3. Begin a turn with `workflow_new_turn`: a `title` plus the first notes, tasks or file changes (a title alone is rejected). The turn a session opens with has no title; name it with `title` on the first `workflow_write_files`, `workflow_write_notes` or `workflow_write_tasks` call. A title cannot be changed later.
4. Put related edits in one `workflow_write_files` call. A call is atomic and recorded immediately; calls in one turn merge into one net diff. Split very large changes into checkpoints.
5. Use `Create` only for new files and `Patch` for edits. Each hunk's `old_text` must match once, or give `start_line` (1-based, in the file before the patch) or scope it with `symbol` to match inside that definition.
6. Verify with `langtool_run_action` (`lint`, `build`, `test`) when the project configures them.
7. Record unfinished work as handover tasks. Before settling the turn, if a mistake cost effort, note the rule that prevents it (what went wrong, what to do instead).
8. Review with `workflow_get_files_diff` and `workflow_get_files_history`; restore earlier content with `Recover`.

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

- Keep one coherent unit of work in the active turn. Settle it by recording what is left with `workflow_write_tasks`; a different unit of work begins in its own turn.
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
