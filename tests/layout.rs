//! The layout rules of `src/`, checked on the files themselves: the layer order, the depth budget
//! and the module file style. The rules are written down in `src/lib.rs`.

use std::fs;
use std::path::{Path, PathBuf};

/// The top-level modules, lowest layer first. A file may use its own layer and the ones before it.
const LAYERS: [&str; 7] = [
    "common",
    "language",
    "toolchain",
    "filesystem",
    "langtool",
    "workflow",
    "server",
];

/// The layers that split into `tools/` and `engine/`.
const SCOPES: [&str; 3] = ["filesystem", "langtool", "workflow"];

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn source_files() -> Vec<PathBuf> {
    fn visit(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut paths: Vec<PathBuf> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                visit(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    visit(&src(), &mut files);
    files
}

/// The path below `src/` with `/` separators: `language/rust/calls.rs`.
fn relative(file: &Path) -> String {
    file.strip_prefix(src())
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

/// The module and the next path segment of every `crate::` path on `line`: `("langtool",
/// Some("tools"))` for `crate::langtool::tools::{A, B}`.
fn crate_paths(line: &str) -> Vec<(String, Option<String>)> {
    line.match_indices("crate::")
        .map(|(at, found)| {
            let mut words = line[at + found.len()..].split("::").map(|word| {
                word.chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>()
            });
            (words.next().unwrap_or_default(), words.next())
        })
        .collect()
}

fn assert_clean(rule: &str, violations: Vec<String>) {
    assert!(
        violations.is_empty(),
        "{rule}:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn files_stay_within_the_depth_budget() {
    let violations = source_files()
        .iter()
        .filter_map(|file| {
            let path = relative(file);
            let depth = path.split('/').count();
            let layer = path.split('/').next().unwrap();
            let budget = if SCOPES.contains(&layer) { 4 } else { 3 };
            (depth > budget).then(|| format!("{path}: {depth} levels, at most {budget}"))
        })
        .collect();
    assert_clean(
        "layers are at most 3 levels below src/, scopes 4",
        violations,
    );
}

#[test]
fn modules_use_mod_rs_files() {
    let violations = source_files()
        .iter()
        .filter(|file| file.file_stem().is_some_and(|stem| stem != "mod"))
        .filter(|file| file.with_extension("").is_dir())
        .map(|file| {
            let path = relative(file);
            format!("{path}: move it to {}/mod.rs", path.trim_end_matches(".rs"))
        })
        .collect();
    assert_clean(
        "a module with submodules is a folder with a mod.rs",
        violations,
    );
}

#[test]
fn dependencies_point_down_the_layers() {
    let rank = |name: &str| LAYERS.iter().position(|layer| *layer == name);
    let mut violations = Vec::new();
    for file in source_files() {
        let path = relative(&file);
        let segments: Vec<&str> = path.split('/').collect();
        let Some(own) = rank(segments[0]) else {
            continue;
        };
        let text = fs::read_to_string(&file).unwrap();
        for (at, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for (module, next) in crate_paths(line) {
                let Some(other) = rank(&module) else {
                    continue;
                };
                if other > own {
                    violations.push(format!(
                        "{path}:{}: `{module}` is above `{}`",
                        at + 1,
                        segments[0]
                    ));
                }
                if segments.get(1) == Some(&"engine") && next.as_deref() == Some("tools") {
                    violations.push(format!(
                        "{path}:{}: an engine does not import `{module}::tools`",
                        at + 1
                    ));
                }
            }
        }
    }
    assert_clean("dependencies point down the layers", violations);
}
