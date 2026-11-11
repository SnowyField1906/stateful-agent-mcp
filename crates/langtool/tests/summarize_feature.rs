//! `langtool_summarize_feature` computes the slice the same way at every zoom; the zoom only sets
//! how much of it is shown.

use stateful_agent_common::ServerConfig;
use stateful_agent_langtool::tools::SummarizeFeatureParams;
use stateful_agent_testkit::TempDir;

use crate::support::Tools;

const WIDGETS: usize = 12;

/// `WIDGETS` features that each call a helper of their own, in files of their own.
fn project() -> TempDir {
    let dir = TempDir::new("summarize_feature_zoom");
    for at in 0..WIDGETS {
        dir.write(
            &format!("src/w{at}.rs"),
            &format!("pub fn render_widget_{at}() {{\n    helper_{at}();\n}}\n"),
        );
        dir.write(
            &format!("src/h{at}.rs"),
            &format!("pub fn helper_{at}() {{}}\n"),
        );
    }
    dir
}

fn feature(dir: &TempDir, zoom: &str) -> String {
    Tools::in_project(dir, ServerConfig::default())
        .langtool_summarize_feature(SummarizeFeatureParams {
            query: Some("render widget".into()),
            symbol: None,
            implicit_path: None,
            zoom: Some(zoom.into()),
            show_gitignored: None,
        })
        .unwrap()
}

/// The items of the section `label` (`core:`), across its rows.
fn section(out: &str, label: &str) -> Vec<String> {
    let mut text = String::new();
    let mut inside = false;
    for line in out.lines() {
        if let Some(first) = line.strip_prefix(label) {
            inside = true;
            text.push_str(first.trim());
        } else if inside && line.starts_with("        ") {
            text.push_str(" · ");
            text.push_str(line.trim());
        } else {
            inside = false;
        }
    }
    text.split(" · ")
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty() && !item.starts_with('…'))
        .collect()
}

#[test]
fn the_number_of_seeds_does_not_depend_on_the_zoom() {
    let dir = project();
    for zoom in ["low", "medium", "high", "max"] {
        let out = feature(&dir, zoom);
        assert!(out.contains(" 8 seeds "), "zoom {zoom}\n{out}");
    }
}

#[test]
fn a_higher_zoom_shows_a_longer_core_with_the_same_start() {
    let dir = project();
    let low = section(&feature(&dir, "low"), "core:");
    let medium = section(&feature(&dir, "medium"), "core:");
    let high = section(&feature(&dir, "high"), "core:");
    assert!(!low.is_empty());
    assert!(medium.len() >= low.len() && high.len() >= medium.len());
    assert_eq!(medium[..low.len()], low[..]);
    assert_eq!(high[..medium.len()], medium[..]);
}
