//! `Patch` hunks whose `old_text` is not in the file byte for byte: the cases a hunk is still
//! placed, and the case that reports every failing hunk at once.

use std::fs;

use stateful_agent_common::ServerConfig;
use stateful_agent_langtool::tools::{FindSymbolsParams, ReadFileParams};
use stateful_agent_workflow::tools::{
    FileOp, NewTurnParams, PatchHunk, RegisterParams, WriteFilesParams,
};

use crate::hunk;
use crate::support::{Session, TempProject};

/// A registered project holding the file `name` with `content`, created by a titled turn so that
/// a later `workflow_new_turn` opens a turn of its own instead of reusing the seeding one.
fn project(dir: &str, name: &str, content: &str) -> (TempProject, Session) {
    let tmp = TempProject::new(dir);
    let state = Session::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();
    state
        .workflow_write_files(WriteFilesParams {
            title: Some("Seed".into()),
            ops: vec![FileOp::Create {
                path: name.into(),
                content: content.into(),
            }],
        })
        .unwrap();
    (tmp, state)
}

fn patch(name: &str, hunks: Vec<PatchHunk>) -> WriteFilesParams {
    WriteFilesParams {
        title: None,
        ops: vec![FileOp::Patch {
            path: name.into(),
            hunks,
        }],
    }
}

fn read(tmp: &TempProject, name: &str) -> String {
    fs::read_to_string(tmp.root.join(name)).unwrap()
}

#[test]
fn a_block_with_the_wrong_indentation_is_seated_and_reported() {
    let (tmp, state) = project(
        "loose-indent",
        "a.rs",
        "fn a() {\n    let x = 1;\n    let y = 2;\n}\n",
    );
    let out = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "let x = 1;\nlet y = 2;\n",
                "let x = 10;\nlet y = 20;\n",
                None,
            )],
        ))
        .unwrap();
    assert_eq!(
        read(&tmp, "a.rs"),
        "fn a() {\n    let x = 10;\n    let y = 20;\n}\n"
    );
    let line = out["updates"][0].as_str().unwrap();
    assert!(
        line.starts_with("Patch a.rs +2 -2 at 2") && line.contains("h1 spacing: indent +4 spaces"),
        "unexpected summary: {line}"
    );
}

#[test]
fn spaces_in_the_quote_match_tabs_in_the_file() {
    let (tmp, state) = project("loose-tabs", "t.txt", "if x {\n\tfoo();\n\tbar();\n}\n");
    let out = state
        .workflow_write_files(patch(
            "t.txt",
            vec![hunk(
                "    foo();\n    bar();\n",
                "    foo();\n    baz();\n",
                None,
            )],
        ))
        .unwrap();
    assert_eq!(read(&tmp, "t.txt"), "if x {\n\tfoo();\n\tbaz();\n}\n");
    let line = out["updates"][0].as_str().unwrap();
    assert!(
        line.contains("indent spaces → tabs"),
        "unexpected summary: {line}"
    );
}

#[test]
fn trailing_blanks_in_the_file_do_not_stop_a_match() {
    let (tmp, state) = project("loose-trailing", "t.txt", "a\nb  \nc\n");
    let out = state
        .workflow_write_files(patch("t.txt", vec![hunk("a\nb\n", "a\nB\n", None)]))
        .unwrap();
    assert_eq!(read(&tmp, "t.txt"), "a\nB\nc\n");
    let line = out["updates"][0].as_str().unwrap();
    assert!(
        line.contains("h1 spacing: trailing whitespace"),
        "unexpected summary: {line}"
    );
}

#[test]
fn resending_an_applied_hunk_is_a_no_op_that_says_so() {
    let (tmp, state) = project("applied", "d.txt", "one\ntwo\nthree\n");
    let hunks = || vec![hunk("two\n", "TWO\nand more\nlines here\n", None)];
    state.workflow_write_files(patch("d.txt", hunks())).unwrap();
    let after = read(&tmp, "d.txt");
    assert_eq!(after, "one\nTWO\nand more\nlines here\nthree\n");

    let out = state.workflow_write_files(patch("d.txt", hunks())).unwrap();
    assert_eq!(read(&tmp, "d.txt"), after);
    let line = out["updates"][0].as_str().unwrap();
    assert!(
        line.contains("already applied"),
        "unexpected summary: {line}"
    );
}

#[test]
fn a_short_replacement_is_never_taken_for_applied() {
    let (tmp, state) = project("short", "d.txt", "one\nTWO\nthree\n");
    let err = state
        .workflow_write_files(patch("d.txt", vec![hunk("two\n", "TWO\n", None)]))
        .unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
    assert_eq!(read(&tmp, "d.txt"), "one\nTWO\nthree\n");
}

#[test]
fn every_failing_hunk_is_reported_at_once_and_nothing_is_written() {
    let original = "a\nb\nc\nd\n";
    let (tmp, state) = project("all-failures", "m.txt", original);
    let err = state
        .workflow_write_files(patch(
            "m.txt",
            vec![
                hunk("a\n", "A\n", None),
                hunk("x\n", "X\n", None),
                hunk("c\n", "C\n", None),
                hunk("y\n", "Y\n", None),
            ],
        ))
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("hunks 2 and 4 of 4 failed")
            && message.contains("nothing was written")
            && message.contains("hunk 2")
            && message.contains("hunk 4"),
        "unexpected error: {message}"
    );
    assert_eq!(read(&tmp, "m.txt"), original);
}

#[test]
fn one_failing_hunk_says_which_others_were_verified() {
    let original = "a\nb\nc\n";
    let (tmp, state) = project("one-failure", "m.txt", original);
    let err = state
        .workflow_write_files(patch(
            "m.txt",
            vec![hunk("a\n", "A\n", None), hunk("x\n", "X\n", None)],
        ))
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("hunk 2 of 2 failed")
            && message.contains("nothing was written")
            && (message.contains("1 verified") || message.contains("1 unverified")),
        "unexpected error: {message}"
    );
    assert_eq!(read(&tmp, "m.txt"), original);
}

#[test]
fn a_loose_quote_that_fits_two_places_needs_start_line() {
    let text = "if x {\n    foo();\n    bar();\n}\nif y {\n    foo();\n    bar();\n}\n";
    let (tmp, state) = project("loose-twice", "b.txt", text);
    let quote = "foo();\nbar();\n";

    let err = state
        .workflow_write_files(patch("b.txt", vec![hunk(quote, "foo();\nbaz();\n", None)]))
        .unwrap_err();
    assert!(err.to_string().contains("matches 2 places"), "{err}");
    assert_eq!(read(&tmp, "b.txt"), text);

    state
        .workflow_write_files(patch(
            "b.txt",
            vec![hunk(quote, "foo();\nbaz();\n", Some(6))],
        ))
        .unwrap();
    assert_eq!(
        read(&tmp, "b.txt"),
        "if x {\n    foo();\n    bar();\n}\nif y {\n    foo();\n    baz();\n}\n"
    );
}

#[test]
fn a_quote_inside_a_multi_line_string_is_not_matched_loosely() {
    let text = "fn s() -> &'static str {\n    \"\n    a\n    keep\n\"\n}\n";
    let (tmp, state) = project("loose-string", "s.rs", text);
    let err = state
        .workflow_write_files(patch("s.rs", vec![hunk("a\nkeep\n", "a\ngone\n", None)]))
        .unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
    assert_eq!(read(&tmp, "s.rs"), text);
}

/// Show the whole of `name` to the agent, as a read tool does.
fn show(state: &Session, name: &str) {
    let params: ReadFileParams =
        serde_json::from_value(serde_json::json!({ "files": [{ "file_path": name }] })).unwrap();
    state.langtool_read_file(params).unwrap();
}

const NUMBERED: &str = "fn a() {\n    let one = 1;\n    let two = 2;\n    let three = 3;\n    let four = 4;\n    let five = 5;\n}\n";

#[test]
fn a_patch_quoted_from_a_read_is_rebased_over_an_outside_edit() {
    let (tmp, state) = project("rebase-outside", "a.rs", NUMBERED);
    show(&state, "a.rs");
    // An editor adds a line on top and rewrites a context line of the patch below.
    fs::write(
        tmp.root.join("a.rs"),
        "// header\nfn a() {\n    let one = 111;\n    let two = 2;\n    let three = 3;\n    let four = 4;\n    let five = 5;\n}\n",
    )
    .unwrap();
    let out = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "    let one = 1;\n    let two = 2;\n    let three = 3;\n    let four = 4;\n    let five = 5;\n",
                "    let one = 1;\n    let two = 2;\n    let three = 3;\n    let four = 44;\n    let five = 5;\n",
                None,
            )],
        ))
        .unwrap();
    assert_eq!(
        read(&tmp, "a.rs"),
        "// header\nfn a() {\n    let one = 111;\n    let two = 2;\n    let three = 3;\n    let four = 44;\n    let five = 5;\n}\n"
    );
    let line = out["updates"][0].as_str().unwrap();
    assert!(line.contains("h1 rebased over outside"), "{line}");
}

#[test]
fn a_patch_over_lines_that_changed_since_the_read_is_refused() {
    let (tmp, state) = project("rebase-conflict", "a.rs", NUMBERED);
    show(&state, "a.rs");
    let edited = NUMBERED.replace("let four = 4;", "let four = 40;");
    fs::write(tmp.root.join("a.rs"), &edited).unwrap();
    let err = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "    let three = 3;\n    let four = 4;\n    let five = 5;\n",
                "    let three = 3;\n    let four = 44;\n    let five = 5;\n",
                None,
            )],
        ))
        .unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
    assert_eq!(read(&tmp, "a.rs"), edited);
}

#[test]
fn an_insertion_quoted_from_a_read_is_seated_between_the_lines_it_named() {
    let (tmp, state) = project("rebase-insert", "m.txt", "l1\nl2\nl3\nl4\nl5\nl6\n");
    show(&state, "m.txt");
    fs::write(
        tmp.root.join("m.txt"),
        "top\nl1 edited\nl2\nl3\nl4\nl5\nl6\n",
    )
    .unwrap();
    let out = state
        .workflow_write_files(patch(
            "m.txt",
            vec![hunk(
                "l1\nl2\nl3\nl4\nl5\n",
                "l1\nl2\nl3\nX\nl4\nl5\n",
                None,
            )],
        ))
        .unwrap();
    assert_eq!(
        read(&tmp, "m.txt"),
        "top\nl1 edited\nl2\nl3\nX\nl4\nl5\nl6\n"
    );
    let line = out["updates"][0].as_str().unwrap();
    assert!(line.contains("rebased over outside"), "{line}");
}

#[cfg(unix)]
#[test]
fn a_patch_quoting_the_text_before_the_formatter_is_rebased_over_it() {
    let tmp = TempProject::new("rebase-formatter");
    let state = Session::new(ServerConfig::default());
    state
        .workflow_register(RegisterParams {
            folder_path: tmp.path_str(),
            show_instructions: Some(false),
            show_last_turn_info: Some(false),
        })
        .unwrap();
    fs::write(
        tmp.root.join(".AGENTS").join("config.toml"),
        "[rust]\nformat = \"sed 's/, */, /g'\"\n",
    )
    .unwrap();
    state
        .workflow_write_files(WriteFilesParams {
            title: None,
            ops: vec![FileOp::Create {
                path: "a.rs".into(),
                content: "fn a(x,y) {\n    let one = 1;\n    let two = 2;\n}\n".into(),
            }],
        })
        .unwrap();
    assert_eq!(
        read(&tmp, "a.rs"),
        "fn a(x, y) {\n    let one = 1;\n    let two = 2;\n}\n"
    );
    let out = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "fn a(x,y) {\n    let one = 1;\n    let two = 2;\n",
                "fn a(x,y) {\n    let one = 1;\n    let two = 22;\n",
                None,
            )],
        ))
        .unwrap();
    assert_eq!(
        read(&tmp, "a.rs"),
        "fn a(x, y) {\n    let one = 1;\n    let two = 22;\n}\n"
    );
    let line = out["updates"][0].as_str().unwrap();
    assert!(line.contains("h1 rebased over formatter"), "{line}");
}

#[test]
fn long_changed_lines_are_kept_when_the_context_around_them_is_misquoted() {
    let text = "fn a() {\n    let first = compute_the_first_value();\n    let second = compute_the_second_value();\n    let third = 3;\n}\n";
    let (tmp, state) = project("rebase-search", "a.rs", text);
    let out = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "    let first = compute_the_first_value();\n    let second = compute_the_second_value();\n    let third = 33;\n",
                "    let first = compute_the_first_value();\n    let second = compute_second();\n    let third = 33;\n",
                None,
            )],
        ))
        .unwrap();
    assert_eq!(
        read(&tmp, "a.rs"),
        "fn a() {\n    let first = compute_the_first_value();\n    let second = compute_second();\n    let third = 3;\n}\n"
    );
    let line = out["updates"][0].as_str().unwrap();
    assert!(
        line.contains("h1 changed lines kept, context differs at 4"),
        "{line}"
    );
}

#[test]
fn short_changed_lines_are_never_placed_by_search() {
    let text = "a\nb\nc\n";
    let (tmp, state) = project("rebase-short", "m.txt", text);
    let err = state
        .workflow_write_files(patch("m.txt", vec![hunk("x\nb\nz\n", "x\nB\nz\n", None)]))
        .unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
    assert_eq!(read(&tmp, "m.txt"), text);
}

const TWICE: &str = "head\nc1\nc2\nTARGET one\nc3\nc4\nmid\nc1\nc2\nTARGET one\nc3\nc4\nend\n";

#[test]
fn a_quote_found_twice_in_the_version_is_placed_at_the_one_nearest_start_line() {
    let (tmp, state) = project("rebase-nearest", "m.txt", TWICE);
    show(&state, "m.txt");
    // Both copies of the quote lost their first line to an editor, so neither is verbatim now.
    let drifted = TWICE.replace("c1\n", "c1 edited\n");
    fs::write(tmp.root.join("m.txt"), &drifted).unwrap();
    let quote = "c1\nc2\nTARGET one\nc3\nc4\n";
    let new = "c1\nc2\nTARGET two\nc3\nc4\n";

    let err = state
        .workflow_write_files(patch("m.txt", vec![hunk(quote, new, None)]))
        .unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
    assert_eq!(read(&tmp, "m.txt"), drifted);

    let out = state
        .workflow_write_files(patch("m.txt", vec![hunk(quote, new, Some(8))]))
        .unwrap();
    assert_eq!(
        read(&tmp, "m.txt"),
        "head\nc1 edited\nc2\nTARGET one\nc3\nc4\nmid\nc1 edited\nc2\nTARGET two\nc3\nc4\nend\n"
    );
    let line = out["updates"][0].as_str().unwrap();
    assert!(line.contains("h1 rebased over outside"), "{line}");
}

#[test]
fn a_patch_quoting_the_text_before_an_earlier_turn_is_rebased_over_that_turn() {
    // Nothing was read, so the only record of the quoted text is the ledger.
    let (tmp, state) = project("rebase-ledger", "m.txt", "l1\nl2\nl3\nl4\nl5\nl6\nl7\n");
    state
        .workflow_new_turn(NewTurnParams {
            title: Some("Edit l3".into()),
            write_notes: None,
            write_tasks: None,
            write_files: Some(vec![FileOp::Patch {
                path: "m.txt".into(),
                hunks: vec![hunk("l3\n", "l3 edited\n", None)],
            }]),
        })
        .unwrap();
    let out = state
        .workflow_write_files(patch(
            "m.txt",
            vec![hunk(
                "l2\nl3\nl4\nl5\nl6\nl7\n",
                "l2\nl3\nl4\nl5\nL6\nl7\n",
                None,
            )],
        ))
        .unwrap();
    assert_eq!(read(&tmp, "m.txt"), "l1\nl2\nl3 edited\nl4\nl5\nL6\nl7\n");
    let line = out["updates"][0].as_str().unwrap();
    assert!(line.contains("h1 rebased over your turn 01.02"), "{line}");
}

#[test]
fn symbol_bodies_shown_by_find_symbols_count_as_seen() {
    let (tmp, state) = project("rebase-symbols", "a.rs", NUMBERED);
    let params: FindSymbolsParams = serde_json::from_value(
        serde_json::json!({ "implicit_path": "a.rs", "include_body": true }),
    )
    .unwrap();
    state.langtool_find_symbols(params).unwrap();
    fs::write(
        tmp.root.join("a.rs"),
        "// header\nfn a() {\n    let one = 111;\n    let two = 2;\n    let three = 3;\n    let four = 4;\n    let five = 5;\n}\n",
    )
    .unwrap();
    let out = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "    let one = 1;\n    let two = 2;\n    let three = 3;\n    let four = 4;\n    let five = 5;\n",
                "    let one = 1;\n    let two = 2;\n    let three = 3;\n    let four = 44;\n    let five = 5;\n",
                None,
            )],
        ))
        .unwrap();
    assert_eq!(
        read(&tmp, "a.rs"),
        "// header\nfn a() {\n    let one = 111;\n    let two = 2;\n    let three = 3;\n    let four = 44;\n    let five = 5;\n}\n"
    );
    let line = out["updates"][0].as_str().unwrap();
    assert!(line.contains("h1 rebased over outside"), "{line}");
}

#[test]
fn a_quote_with_squeezed_inner_blanks_is_seated_and_the_context_keeps_its_spacing() {
    let (tmp, state) = project(
        "squeezed",
        "a.rs",
        "fn a() {\n    let x   =   1;\n    let y = 2;\n}\n",
    );
    let out = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "    let x = 1;\n    let y = 2;\n",
                "    let x = 1;\n    let y = 20;\n",
                None,
            )],
        ))
        .unwrap();
    assert_eq!(
        read(&tmp, "a.rs"),
        "fn a() {\n    let x   =   1;\n    let y = 20;\n}\n"
    );
    let line = out["updates"][0].as_str().unwrap();
    assert!(line.contains("h1 spacing: inner whitespace"), "{line}");
}

#[test]
fn blanks_inside_a_string_are_never_squeezed() {
    let text = "fn a() {\n    let s = \"a  b\";\n    let t = 1;\n}\n";
    let (tmp, state) = project("squeezed-string", "a.rs", text);
    let err = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "    let s = \"a b\";\n    let t = 1;\n",
                "    let s = \"a b\";\n    let t = 2;\n",
                None,
            )],
        ))
        .unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
    assert_eq!(read(&tmp, "a.rs"), text);
}

const GAPPED: &str = "fn a() {\n    one();\n    two();\n    three();\n    four();\n}\n";

#[test]
fn a_placeholder_line_stands_for_the_lines_between_head_and_tail() {
    let (tmp, state) = project("elide-ok", "a.rs", GAPPED);
    let out = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "    one();\n    // ... existing code ...\n    four();\n",
                "    one();\n    // ... existing code ...\n    FOUR();\n",
                None,
            )],
        ))
        .unwrap();
    assert_eq!(
        read(&tmp, "a.rs"),
        "fn a() {\n    one();\n    two();\n    three();\n    FOUR();\n}\n"
    );
    assert!(
        out.get("placeholder_warnings").is_none(),
        "an expanded placeholder is not written as text: {out}"
    );
}

#[test]
fn a_placeholder_only_in_the_quote_is_diagnosed_and_nothing_is_written() {
    let (tmp, state) = project("elide-lost", "a.rs", GAPPED);
    let err = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "    one();\n    // ... existing code ...\n    four();\n",
                "    one();\n    FOUR();\n",
                None,
            )],
        ))
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("elision") && message.contains("nothing was written"),
        "unexpected error: {message}"
    );
    assert_eq!(read(&tmp, "a.rs"), GAPPED);
}

#[test]
fn a_placeholder_line_written_as_text_is_flagged() {
    let (tmp, state) = project("placeholder-flag", "a.rs", "fn a() {\n    let x = 1;\n}\n");
    let out = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "    let x = 1;\n",
                "    let x = 2;\n    // ... rest of function ...\n",
                None,
            )],
        ))
        .unwrap();
    assert!(read(&tmp, "a.rs").contains("// ... rest of function ..."));
    let warnings = out["placeholder_warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("no placeholder_warnings in {out}"));
    assert_eq!(warnings.len(), 1, "{out}");
    assert!(
        warnings[0].as_str().unwrap().contains("rest of function"),
        "{out}"
    );
}

#[test]
fn a_quote_of_lines_an_earlier_hunk_rewrote_is_diagnosed_as_overlap() {
    let original = "a\nb\nc\nd\n";
    let (tmp, state) = project("overlap", "m.txt", original);
    let err = state
        .workflow_write_files(patch(
            "m.txt",
            vec![
                hunk("b\nc\n", "B\nC\n", None),
                hunk("c\nd\n", "x\ny\n", None),
            ],
        ))
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("overlap") && message.contains("rewrote") && message.contains("hunk 1"),
        "unexpected error: {message}"
    );
    assert_eq!(read(&tmp, "m.txt"), original);
}

#[test]
fn an_exact_quote_found_twice_says_what_tells_the_places_apart() {
    let text = "if x {\n    foo();\n}\nif y {\n    foo();\n}\n";
    let (tmp, state) = project("exact-twice", "b.txt", text);
    let err = state
        .workflow_write_files(patch(
            "b.txt",
            vec![hunk("    foo();\n", "    bar();\n", None)],
        ))
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("ambiguous")
            && message.contains("matches 2 places")
            && message.contains("at line"),
        "unexpected error: {message}"
    );
    assert_eq!(read(&tmp, "b.txt"), text);
}

#[test]
fn a_refused_patch_names_who_changed_the_lines() {
    let (tmp, state) = project("stale-cause", "a.rs", NUMBERED);
    show(&state, "a.rs");
    let edited = NUMBERED.replace("let four = 4;", "let four = 40;");
    fs::write(tmp.root.join("a.rs"), &edited).unwrap();
    let err = state
        .workflow_write_files(patch(
            "a.rs",
            vec![hunk(
                "    let three = 3;\n    let four = 4;\n    let five = 5;\n",
                "    let three = 3;\n    let four = 44;\n    let five = 5;\n",
                None,
            )],
        ))
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("outside") && message.contains("nothing was written"),
        "unexpected error: {message}"
    );
    assert_eq!(read(&tmp, "a.rs"), edited);
}
