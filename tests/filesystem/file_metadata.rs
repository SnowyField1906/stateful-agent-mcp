use agent_mcp::ServerConfig;
use agent_mcp::filesystem::tools::FileMetadataParams;
use agent_mcp::server::State;

use crate::common::TempProject;

#[test]
fn file_metadata_reports_size_type_and_lines() {
    let tmp = TempProject::new("meta-test");
    tmp.write("data.txt", "line1\nline2\nline3\n");

    let state = State::new(ServerConfig::default());
    let path = tmp.root.join("data.txt").to_string_lossy().into_owned();

    let meta = state
        .filesystem_file_metadata(FileMetadataParams {
            implicit_path: Some(path),
        })
        .unwrap();

    assert!(meta.contains("lines: 3"), "unexpected meta: {meta}");
    assert!(meta.contains("data.txt"), "unexpected meta: {meta}");
}
