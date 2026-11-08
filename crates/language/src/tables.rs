//! Project manifests read as TOML tables, for the languages whose manifests are TOML.

use std::path::Path;

use toml::Table;

/// The manifest at `path` as a table; `None` when it is missing or is not valid TOML.
pub fn read(path: &Path) -> Option<Table> {
    std::fs::read_to_string(path).ok()?.parse().ok()
}

/// The table at the key path `keys` below `table`: `["tool", "uv", "workspace"]` is
/// `[tool.uv.workspace]`.
pub fn section<'a>(table: &'a Table, keys: &[&str]) -> Option<&'a Table> {
    let mut current = table;
    for key in keys {
        current = current.get(*key)?.as_table()?;
    }
    Some(current)
}
