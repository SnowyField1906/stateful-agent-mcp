use stateful_agent_common::fs::walk::{Kind, SortBy};
use stateful_agent_common::{PageParams, Range, ServerConfig};
use stateful_agent_filesystem::tools::LsParams;
use stateful_agent_testkit::TempDir;

use crate::support::Tools;

#[test]
fn ls_accepts_absolute_paths_without_a_registered_project() {
    let tmp = TempDir::new("absolute-ls");
    tmp.write("data/defs.lua", "one\ntwo\n");
    let state = Tools::new(ServerConfig::default());

    let dir = tmp.root.join("data").to_string_lossy().into_owned();
    let listing = state
        .filesystem_ls(LsParams {
            folder_path: Some(dir),
            ..Default::default()
        })
        .unwrap();
    assert!(
        listing.contains("defs.lua"),
        "unexpected listing: {listing}"
    );

    let err = state.filesystem_ls(LsParams::default()).unwrap_err();
    assert!(
        err.to_string().contains("workflow_register"),
        "unexpected error: {err}"
    );
}

#[test]
fn entries_accept_a_tail_range() {
    let tmp = TempDir::new("tail-entries");
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        tmp.write(name, "x\n");
    }
    let state = Tools::new(ServerConfig::default());

    let listing = state
        .filesystem_ls(LsParams {
            folder_path: Some(tmp.path_str()),
            page: PageParams {
                entries: Some(Range {
                    start: -2,
                    end: None,
                }),
            },
            ..Default::default()
        })
        .unwrap();
    assert!(
        listing.contains("c.txt") && listing.contains("d.txt") && !listing.contains("b.txt"),
        "unexpected listing: {listing}"
    );
}

#[test]
fn ls_accepts_strongly_typed_kind_and_sort_by() {
    let tmp = TempDir::new("ls-typed");
    tmp.write("b.txt", "123\n");
    tmp.write("a.txt", "123456\n");
    let state = Tools::new(ServerConfig::default());

    let listing = state
        .filesystem_ls(LsParams {
            folder_path: Some(tmp.path_str()),
            kind: Some(Kind::File),
            sort_by: Some(SortBy::Size),
            ..Default::default()
        })
        .unwrap();
    // a.txt is 7 bytes, b.txt is 4 bytes. Largest first when sorted by size.
    let pos_a = listing.find("a.txt").expect("a.txt present");
    let pos_b = listing.find("b.txt").expect("b.txt present");
    assert!(pos_a < pos_b, "sorted by size: {listing}");
}
