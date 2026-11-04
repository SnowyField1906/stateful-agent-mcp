//! Tool descriptions shown to the agent. Keep them short, factual and free of emphasis.

pub const WORKFLOW_REGISTER: &str = "Register a project (`folder_path`) and start or reuse a session. Call this first. Optional booleans: `show_instructions`, `show_last_turn_info`. Returns the active turn ID, the usage guide, the project's own instructions, and the previous turn's notes, handover tasks and update summary.";

pub const WORKFLOW_NEW_TURN: &str = "Begin the next turn and run its first `write_notes`, `write_tasks` and `write_files` operations there; `title` names the turn and cannot be changed later. At least one operation is required, so a title alone is rejected. An active turn that has recorded nothing is used instead of opening another.";

pub const WORKFLOW_WRITE_FILES: &str = r#"Apply file operations (`ops: [...]`) atomically and record them in the active turn. Paths are project-relative.
- `{"op":"Create","path":"...","content":"..."}`: create a file; fails if it exists.
- `{"op":"Patch","path":"...","hunks":[{"old_text":"...","new_text":"...","start_line":1}]}`: replace text; `old_text` must match once or use a 1-based `start_line`.
- `{"op":"Move","path":"...","to_path":"..."}`: rename a file or directory.
- `{"op":"Delete","path":"..."}`: remove a file or directory.
- `{"op":"Recover","id":"...","to_path":"..."}`: restore from an update ID or turn/session/range, optionally limited to a file or directory prefix.
If the project configures a formatter for a language, files of that language are formatted automatically after the write, so a later `Patch` must match the formatted text. Calls within one turn merge into one net diff. Batch related edits; split large changes only as checkpoints. Returns one summary line per operation under `updates`, such as `Patch src/a.rs +3 -1 at 12`."#;

pub const LANGTOOL_RUN_ACTION: &str = "Run a command the user configured: `language` (`rust`, `python`, ...) and `action` (`lint`, `build`, `test`, `install`, `remove`). `workflow_register` lists the configured actions under `language_capabilities`; an entry such as `test {args}` accepts `args` (for example `--help` to see what it accepts). Returns the command, exit status, stdout and stderr, with a 10-minute limit.";

pub const WORKFLOW_WRITE_NOTES: &str = r#"Modify active-turn notes (`ops: [{"op":"Append", ...}, {"op":"Edit", ...}, {"op":"Delete", ...}]`). Notes are optional memory for later turns: keep only concise durable facts or decisions with their reason or implication. Omit progress, tool/build status, duplicates, and facts already in file updates; leave notes empty when nothing qualifies. IDs within one call refer to the numbering at the start of the call, so several deletes never shift each other; the response lists the notes that remain. `Append` and `Edit` accept `ref_notes` (IDs of related notes) to point to earlier notes instead of repeating them."#;

pub const WORKFLOW_WRITE_TASK: &str = r#"Modify handover tasks of the active turn (`ops: [{"op":"Append", task_content, user_explicit?}, {"op":"Edit", task_id, task_content, user_explicit?}, {"op":"Delete", task_id}, {"op":"RollOver", task_id, task_content?, user_explicit?}]`). Handover tasks are short-term work for the next turn or session. Set `user_explicit` for user-requested tasks. RollOver copies an unfinished task from an earlier turn, so its content need not be copied by hand."#;

pub const WORKFLOW_GET_NOTES: &str = "Read notes of past turns. `implicit_ids` is a selector or list of selectors: session `01`, turn `01.02`, inclusive range `01.01..01.03` (`01.02..` runs to the active turn), `HEAD`, `HEAD~1`, or a note ID `01.02.note.01`.";

pub const WORKFLOW_GET_TASKS: &str = "Read handover tasks of past turns. `implicit_ids` takes the same selectors as `workflow_get_notes`, with task IDs like `01.02.task.01`.";

pub const WORKFLOW_GET_FILES_DIFF: &str = "Net file changes of a turn, session, range or update (`implicit_id`) as a unified diff: a summary line per operation (`Create`, `Patch`, `Move`, `Delete`) followed by `@@ -old,n +new,m @@` hunks with `-`, `+` and space-prefixed lines. A `Delete` shows no content; use `Recover` to restore files. A reversed range (`01.05..01.02`) gives the inverse diff. Optional `implicit_paths` limits the result to files or directories.";

pub const WORKFLOW_GET_FILES_HISTORY: &str = "Which turns changed which files, newest first, without content (use `workflow_get_files_diff` for content). Optional `implicit_id` limits the turns and `implicit_paths` limits the files; renames are followed.";

pub const FILESYSTEM_LS: &str = "List a directory (`folder_path`, default: project root, or any absolute path on disk, which needs no registered project) with sizes, directories first. Respects `.gitignore`; dotfiles are hidden. Options: `recursive`, `glob`, `kind`, `sort_by`, `reverse`, `long`, `show_gitignored`, and paging with `entries`.";

pub const FILESYSTEM_FIND: &str = "Find files under `folder_path` (default: project root, or any absolute directory on disk, which needs no registered project) by `glob` (gitignore syntax, for example `*.rs` or `src/**/*.ts`), `kind`, `size`, `sort_by`, `reverse`, `show_gitignored`, and paging with `entries`.";

pub const LANGTOOL_READ_FILE: &str = "Read 1–32 text files in one call (`files: [{file_path, lines?, numbered?, fold_level?}]`; any absolute path is read directly without a registered project). Each file has its own optional 1-based inclusive `lines` range (default first 400 lines), `numbered` setting, and optional `fold_level` (for Rust and Python files, folds AST blocks nested deeper than `fold_level` into `A..B<tab>text` rows; `0` shows only top-level declarations). `at_implicit_id` reads project files as they were at the end of a past turn.";

pub const FILESYSTEM_GREP: &str = "Regex search (`query`, Rust syntax, or literal text with `literal: true`) in `implicit_path` (file or directory, default: project root, or any absolute path on disk without a registered project). Options: `glob`, `case_insensitive`, `context_lines`, `output` (`content`, `files` or `count`), `show_gitignored`, and `entries`. Binary files and files over 8 MiB are skipped.";

pub const LANGTOOL_FIND_SYMBOLS: &str = "List definitions (functions, structs, enums, traits, impls, modules, classes, ...) in a file or below a directory (`implicit_path`, default: project root, or any absolute path on disk), parsed with Tree-sitter; supports Rust (`.rs`) and Python (`.py`, `.pyi`). Output is grouped by file; each line gives `start..end`, kind with its enclosing scope, and the signature. Options: `query` (name filter), `kinds` (`{\"rust\": [...], \"python\": [...]}`), `show_gitignored`, and `entries`. Use the line ranges with `langtool_read_file`.";

pub const LANGTOOL_FIND_REFERENCES: &str = "Where a name is defined and used in Rust or Python code, parsed with Tree-sitter so comments and string literals are ignored. `symbol` plus `implicit_path` (file or directory, default: project root, or any absolute path on disk). Output is grouped by file: `line:text` for a use, `line:def:text` for a definition, `line:alias NAME:text` for a use through an import alias. Options: `show_gitignored`, `entries`. Matching is by name only, so same-named items in different scopes are not told apart; `langtool_find_symbols` shows the scopes.";

pub const LANGTOOL_VIEW_CALLGRAPH: &str = "Call graph of a function or method in Rust or Python code, parsed with Tree-sitter: what it calls (`outgoing`) and what calls it (`incoming`). `symbol` is the function name; `implicit_path` (file or directory, default: project root, or any absolute path on disk) is the code that gets indexed, so callers outside it are not found. Options: `direction` (`incoming`, `outgoing` or `both`, default), `tree_depth` (call levels, default 1, max 3), `file_path` and `line` to pick one definition when several share the name, `show_gitignored`. Output is a tree per definition: `name (in scope)<tab>file:start..end`, indented per level, with `(external)` for a call that has no definition in the indexed code and `(cycle)` where a call leads back. Calls are matched by name only, so same-named functions in different files are all listed.";

pub const FILESYSTEM_FILE_METADATA: &str = "Size, permissions and modification time of `implicit_path` (file or directory, relative to the project root or any absolute path on disk). For text files also line count, line endings and final newline, plus the last turn that changed the file.";
