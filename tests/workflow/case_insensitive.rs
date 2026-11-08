//! Paths are spelled the way the disk stores them, so a filesystem that ignores case (APFS and
//! HFS+ on macOS, NTFS on Windows) cannot give the ledger two keys for one file.

use std::fs;

use agent_mcp::ServerConfig;
use agent_mcp::workflow::tools::{FileOp, GetFilesDiffParams, WriteFilesParams};

use crate::common::TempProject;
use crate::hunk;

/// Whether the directory of `tmp` finds `src` as `SRC`.
fn ignores_case(tmp: &TempProject) -> bool {
    tmp.root.join("SRC").exists()
}

fn write(state: &agent_mcp::server::State, ops: Vec<FileOp>) -> String {
    state
        .workflow_write_files(WriteFilesParams { title: None, ops })
        .unwrap()
        .to_string()
}

fn patch(path: &str, old: &str, new: &str) -> FileOp {
    FileOp::Patch {
        path: path.into(),
        hunks: vec![hunk(old, new, None)],
    }
}

#[test]
fn one_file_written_in_two_cases_is_one_entry_of_the_ledger() {
    let tmp = TempProject::new("case-insensitive");
    tmp.write("src/lib.rs", "one\n");
    if !ignores_case(&tmp) {
        eprintln!("skipped: this filesystem tells names apart by case");
        return;
    }
    let state = tmp.open(ServerConfig::default());

    let out = write(
        &state,
        vec![
            patch("SRC/LIB.RS", "one\n", "two\n"),
            patch("src/Lib.rs", "two\n", "three\n"),
        ],
    );
    assert!(out.contains("src/lib.rs"), "{out}");
    assert!(!out.contains("LIB") && !out.contains("Lib.rs"), "{out}");
    assert_eq!(
        fs::read_to_string(tmp.root.join("src/lib.rs")).unwrap(),
        "three\n"
    );

    let diff = state
        .workflow_get_files_diff(GetFilesDiffParams {
            implicit_id: "01.01".into(),
            implicit_paths: Some(vec!["SRC/Lib.rs".into()]),
        })
        .unwrap();
    assert!(diff.contains("src/lib.rs"), "{diff}");
    assert!(!diff.contains("LIB") && !diff.contains("Lib.rs"), "{diff}");
    assert!(diff.contains("-one") && diff.contains("+three"), "{diff}");
}

#[test]
fn a_file_created_in_a_batch_is_one_entry_under_another_case() {
    let tmp = TempProject::new("case-pending");
    tmp.write("src/lib.rs", "one\n");
    if !ignores_case(&tmp) {
        eprintln!("skipped: this filesystem tells names apart by case");
        return;
    }
    let state = tmp.open(ServerConfig::default());

    let out = write(
        &state,
        vec![
            FileOp::Create {
                path: "docs/Mixed.txt".into(),
                content: "one\n".into(),
            },
            patch("DOCS/MIXED.TXT", "one\n", "two\n"),
            patch("docs/mixed.txt", "two\n", "three\n"),
        ],
    );
    assert!(out.contains("docs/Mixed.txt"), "{out}");
    assert!(
        !out.contains("MIXED") && !out.contains("mixed.txt"),
        "{out}"
    );
    assert_eq!(
        fs::read_to_string(tmp.root.join("docs/Mixed.txt")).unwrap(),
        "three\n"
    );
    let listed: Vec<String> = fs::read_dir(tmp.root.join("docs"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(listed, ["Mixed.txt"]);
}

#[test]
fn new_names_keep_their_spelling_under_folders_spelled_as_stored() {
    let tmp = TempProject::new("case-new-names");
    tmp.write("src/lib.rs", "one\n");
    if !ignores_case(&tmp) {
        eprintln!("skipped: this filesystem tells names apart by case");
        return;
    }
    let state = tmp.open(ServerConfig::default());

    let create = |path: &str| FileOp::Create {
        path: path.into(),
        content: "fn a() {}\n".into(),
    };
    let out = write(&state, vec![create("Src/New.rs")]);
    assert!(out.contains("src/New.rs"), "{out}");
    assert!(tmp.root.join("src/New.rs").is_file());

    // The same file in another case is that file, not a new one.
    let err = state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![create("SRC/NEW.RS")],
        })
        .unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");

    // A file moves to the name that was written, in the folder as stored.
    let out = write(
        &state,
        vec![FileOp::Move {
            path: "SRC/Lib.RS".into(),
            to_path: "SRC/Moved.rs".into(),
        }],
    );
    assert!(
        out.contains("src/lib.rs") && out.contains("src/Moved.rs"),
        "{out}"
    );
    assert!(tmp.root.join("src/Moved.rs").is_file());
}

#[test]
fn names_that_differ_by_case_stay_apart_where_the_disk_tells_them_apart() {
    let tmp = TempProject::new("case-sensitive");
    tmp.write("src/lib.rs", "one\n");
    if ignores_case(&tmp) {
        eprintln!("skipped: this filesystem ignores case");
        return;
    }
    let state = tmp.open(ServerConfig::default());
    let out = write(
        &state,
        vec![FileOp::Create {
            path: "src/Lib.rs".into(),
            content: "two\n".into(),
        }],
    );
    assert!(out.contains("src/Lib.rs"), "{out}");
    assert_eq!(
        fs::read_to_string(tmp.root.join("src/lib.rs")).unwrap(),
        "one\n"
    );
}
