# MCP usage guide

## Concepts

- **Session**: starts with `register_project`. **Turn**: one unit of work inside a session, ID `SS.TT` (for example `01.03`). Only the active turn is writable; closed turns are immutable.
- **Notes**: concise durable facts or decisions a later turn needs. Include the reason or implication. Never log progress, tool/build status, duplicates, or information already in file updates. Leave notes empty when nothing qualifies. When a note builds on an earlier one, refer to it with `ref_notes` instead of repeating it.
- **Handover tasks**: short-term work left for the next turn or session. They are not the project's task list. Mark tasks the user asked for with `user_explicit: true`. Carry an unfinished task forward with `RollOver` instead of copying its content.
- **File updates**: `write_files` takes `Create`, `Move`, `Patch`, `Delete` and `Recover`. Inputs use `{op, ...fields}`. Results list one summary line per operation (`Patch src/a.rs +3 -1`), and `get_files_diff` returns a unified diff.

## Working in a turn

1. Read the previous turn's notes and handover tasks from `register_project`.
2. Inspect with `fs_ls`, `fs_find`, `fs_grep`, `fs_read_files`. Pass a `files` list with `{path | path_absolute, lines?, numbered?}` per file to read related files in one call; read only what the task needs. `lines` and the listing parameter `entries` take the same `START..END` range syntax (`10..50`, `10..`, `..50`, a single number, or `-30..` for the last 30), and each result prints the next range to ask for. `path_absolute` reads any absolute path without a registered project.
3. Give a turn its title when it begins: pass `title` to `create_new_turn` together with the turn's first notes, tasks or file changes (a title alone is rejected). The title cannot be changed later.
4. Put related edits in one `write_files` call by default. A call is atomic and recorded immediately, and repeated calls in one turn merge into one net diff. For very long files (patch section by section) or large multi-file changes, split across several calls as checkpoints.
5. Use `Create` only for new files and `Patch` for all edits to existing files. Each hunk's `old_text` must match once (or be disambiguated by `start_line`).
6. Record unfinished work in handover tasks.
7. Review with `get_files_diff` and `get_files_history`. Restore earlier content with `Recover`.

## Style

Write plain, precise English: facts, constraints, reasons. Avoid emphasis, hype and alarming wording.
Keep note and task IDs inside `.AGENTS/`; never reference them in project files or code comments.

## Session-based mode

The session may stop at any time; leave a clear handover for unfinished work.

- Keep one coherent unit of work in the active turn. Settle it by recording what is left in `write_tasks`; a different unit of work begins in its own turn.
- Do not start a new turn just because the session continues or the same unit has more work.
- Mark user-requested tasks `user_explicit: true` and roll unfinished tasks of earlier turns over with `RollOver`.
