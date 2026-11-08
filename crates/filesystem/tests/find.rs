use stateful_agent_common::ServerConfig;
use stateful_agent_common::fs::walk::{Kind, SortBy};
use stateful_agent_filesystem::tools::FindParams;
use stateful_agent_testkit::TempDir;

use crate::support::Tools;

#[test]
fn find_locates_files_by_glob_kind_and_sorting() {
    let tmp = TempDir::new("find-test");
    tmp.write("src/lib.rs", "pub fn a() {}\n");
    tmp.write("src/main.rs", "fn main() { let x = 1; }\n");
    tmp.write("README.md", "# Hello\n");

    let state = Tools::new(ServerConfig::default());

    // Glob filter
    let found = state
        .filesystem_find(FindParams {
            folder_path: Some(tmp.path_str()),
            glob: Some(vec!["*.rs".into()]),
            ..Default::default()
        })
        .unwrap();
    assert!(found.contains("lib.rs"), "unexpected find: {found}");
    assert!(found.contains("main.rs"), "unexpected find: {found}");
    assert!(!found.contains("README.md"), "unexpected find: {found}");

    // Kind filter
    let dirs = state
        .filesystem_find(FindParams {
            folder_path: Some(tmp.path_str()),
            kind: Some(Kind::Dir),
            ..Default::default()
        })
        .unwrap();
    assert!(dirs.contains("src"), "unexpected dirs: {dirs}");
    assert!(!dirs.contains("lib.rs"), "unexpected dirs: {dirs}");

    // Sort by size
    let sorted = state
        .filesystem_find(FindParams {
            folder_path: Some(tmp.path_str()),
            sort_by: Some(SortBy::Size),
            ..Default::default()
        })
        .unwrap();
    assert!(sorted.contains("src/main.rs"), "unexpected sort: {sorted}");
}
