//! `workflow_write_files`: applies a batch of file operations atomically and records the turn's
//! net changes. The batch itself is `engine::files`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::common::Result;
use crate::common::params::lenient_vec;
use crate::workflow::Project;
use crate::workflow::engine::ops::FileOp;

pub const DESCRIPTION: &str = r#"Apply file operations atomically in the active turn. Paths are project-relative.
- `Create {path, content}`: new file; fails if it exists.
- `Patch {path, hunks: [{old_text, new_text, start_line?, symbol?}]}`: replace text. `old_text` must match exactly once; otherwise give `start_line` (1-based, in the file before the patch) or `symbol` (`name`, `Type::name`; Rust and Python) to search only inside that definition.
- `ReplaceSymbol {path, symbol, content}`: replace the whole definition `symbol` (`name`, `Type::name`; Rust and Python) with `content`, which is moved as a block to the definition's indentation. Doc comments and attributes above it stay unless `content` starts with its own.
- `InsertBeforeSymbol` / `InsertAfterSymbol {path, symbol, content}`: add `content` above (with the symbol's docs below it) or below that definition, separated by a blank line.
- `RenameSymbol {symbol, new_name, path?}`: rename the identifiers of `symbol` (`name`, or `Type::name` to keep to that type) in the Rust and Python files below `path` (default: the whole project, gitignored files excluded). Comments, strings and uses through an import alias are left alone. Matching is by name: a qualified `symbol` keeps to identifiers inside that type and lines that name it (`store.put()` is not renamed), and the result counts the same-named identifiers left alone. Fails when `new_name` is not a plain identifier or the rename would cause a syntax error.
- `Move {path, to_path}`: rename a file or directory.
- `Delete {path}`: remove a file or directory.
- `Recover {id, to_path?}`: restore from an update ID (`01.02.update.src/a.rs`) or a turn, session or range selector; `to_path` limits it to a file or directory prefix.
A formatter configured for a language rewrites the files written in it, so a later `Patch` must match the formatted text. Repeated calls in one turn merge into one net diff. Optional `title` names the active turn when it has none; a titled turn keeps its title. Returns one line per operation, such as `Patch src/a.rs +3 -1 at 12`."#;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteFilesParams {
    /// Operations, applied in order as one atomic batch. Each uses `{op: "Create", ...}` shape.
    #[serde(deserialize_with = "lenient_vec")]
    #[schemars(with = "Vec<FileOp>")]
    pub ops: Vec<FileOp>,
    /// Title for the active turn if it has none (single line). A titled turn keeps its title.
    /// Applied only when the batch succeeds.
    #[serde(default)]
    pub title: Option<String>,
}

pub fn write_files(project: &mut Project, p: WriteFilesParams) -> Result<Value> {
    write_files_with_paths(project, p).map(|(out, _)| out)
}

pub(crate) fn write_files_with_paths(
    project: &mut Project,
    p: WriteFilesParams,
) -> Result<(Value, Vec<String>)> {
    let external = project.sync_external_edits()?;
    let batch = project.apply_files(&p.ops)?;
    let title = project.name_active_turn(p.title.as_deref())?;
    let changed_paths = batch.changed_paths.clone();
    let mut out = json!(batch);
    out["turn_id"] = json!(project.active.to_string());
    if let Some(external) = &external {
        out["external_edits"] = external.report();
    }
    title.report(project.active_turn()?, &mut out);
    Ok((out, changed_paths))
}
