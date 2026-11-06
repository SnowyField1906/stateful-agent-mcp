use agent_mcp::ServerConfig;
use agent_mcp::common::{PageParams, Range};
use agent_mcp::filesystem::tools::LsParams;
use agent_mcp::langtool::tools::{ReadFileEntry, ReadFileParams};
use agent_mcp::server::State;

use crate::common::TempProject;

#[test]
fn read_tools_accept_absolute_paths_without_a_registered_project() {
    let tmp = TempProject::new("absolute-read");
    tmp.write("data/defs.lua", "one\ntwo\n");
    let state = State::new(ServerConfig::default());

    let file = tmp
        .root
        .join("data/defs.lua")
        .to_string_lossy()
        .into_owned();
    let read = state
        .langtool_read_file(ReadFileParams {
            files: vec![ReadFileEntry {
                file_path: file,
                numbered: Some(false),
                ..ReadFileEntry::default()
            }],
            at_implicit_id: None,
        })
        .unwrap();
    assert!(read.contains("one\ntwo\n"), "unexpected read: {read}");

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
    let tmp = TempProject::new("tail-entries");
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        tmp.write(name, "x\n");
    }
    let state = State::new(ServerConfig::default());

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
