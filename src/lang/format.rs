//! Automatic formatting of files written through `write_files`.
//!
//! A language's `format` command is used in one of three ways:
//! - it contains `{files}`: run once with all files of that language;
//! - it contains `{file}`: run once per file;
//! - otherwise: the file's text is piped to stdin and stdout replaces the file.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use crate::session::store::atomic_write;

use super::config::LanguageConfig;
use super::exec::{Captured, quote, run_shell};
use super::language_for_path;

pub const FORMAT_TIMEOUT: Duration = Duration::from_secs(60);

/// Outcome of one formatting pass.
#[derive(Debug, Default)]
pub struct FormatReport {
    /// Files whose content the formatter changed.
    pub formatted: Vec<String>,
    /// Problems; a failing formatter never fails the write.
    pub warnings: Vec<String>,
}

/// Format `paths` (project-relative, existing on disk) with the configured formatters.
pub fn format_paths(config: &LanguageConfig, root: &Path, paths: &[String]) -> FormatReport {
    let mut report = FormatReport::default();
    let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for path in paths {
        if let Some(lang) = language_for_path(path) {
            groups.entry(lang.name).or_default().push(path);
        }
    }
    for (name, files) in groups {
        let Some(command) = config.language(name).and_then(|t| t.format.as_deref()) else {
            continue;
        };
        let before: BTreeMap<&str, String> = files
            .iter()
            .filter_map(|&f| Some((f, std::fs::read_to_string(root.join(f)).ok()?)))
            .collect();
        let existing: Vec<&str> = before.keys().copied().collect();
        if command.contains("{files}") {
            let joined = existing.iter().map(|f| quote(f)).collect::<Vec<_>>().join(" ");
            run_template(root, name, command, &command.replace("{files}", &joined), &existing, &before, &mut report);
        } else if command.contains("{file}") {
            for &f in &existing {
                let full = command.replace("{file}", &quote(f));
                run_template(root, name, command, &full, &[f], &before, &mut report);
            }
        } else {
            for &f in &existing {
                format_stdin(root, name, command, f, &before[f], &mut report);
            }
        }
    }
    report
}

/// Run a command that edits the files in place, then report which ones changed.
fn run_template(
    root: &Path,
    name: &str,
    configured: &str,
    full: &str,
    files: &[&str],
    before: &BTreeMap<&str, String>,
    report: &mut FormatReport,
) {
    match run_shell(full, root, None, FORMAT_TIMEOUT) {
        Ok(out) if out.success() => {
            for &f in files {
                let now = std::fs::read_to_string(root.join(f)).ok();
                if now.is_some() && now.as_ref() != before.get(f) {
                    report.formatted.push(f.to_string());
                }
            }
        }
        Ok(out) => report.warnings.push(failure(name, configured, &out)),
        Err(e) => report.warnings.push(e.to_string()),
    }
}

/// Pipe a file through a stdin/stdout formatter and replace it when the output differs.
fn format_stdin(
    root: &Path,
    name: &str,
    command: &str,
    file: &str,
    old: &str,
    report: &mut FormatReport,
) {
    match run_shell(command, root, Some(old.as_bytes().to_vec()), FORMAT_TIMEOUT) {
        Ok(out) if out.success() => match String::from_utf8(out.stdout) {
            Ok(new) if !new.is_empty() || old.is_empty() => {
                if new != old {
                    match atomic_write(&root.join(file), new.as_bytes()) {
                        Ok(()) => report.formatted.push(file.to_string()),
                        Err(e) => report.warnings.push(format!("{name} formatter: {e}")),
                    }
                }
            }
            _ => report.warnings.push(format!(
                "{name} formatter `{command}` gave no usable output for {file}; it must read stdin and write stdout, or use `{{file}}`"
            )),
        },
        Ok(out) => report.warnings.push(failure(name, command, &out)),
        Err(e) => report.warnings.push(e.to_string()),
    }
}

fn failure(name: &str, command: &str, out: &Captured) -> String {
    let what = if out.timed_out {
        "timed out".to_string()
    } else {
        format!("exited with {}", out.status.map_or("a signal".into(), |c| c.to_string()))
    };
    let stderr = String::from_utf8_lossy(&out.stderr);
    let detail: String = stderr.trim().chars().take(400).collect();
    if detail.is_empty() {
        format!("{name} formatter `{command}` {what}")
    } else {
        format!("{name} formatter `{command}` {what}: {detail}")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("agent-mcp-fmt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn stdin_formatter_rewrites_matching_files_only() {
        let dir = temp("stdin");
        std::fs::write(dir.join("a.rs"), "abc\n").unwrap();
        std::fs::write(dir.join("b.md"), "abc\n").unwrap();
        let cfg = LanguageConfig::parse("[rust]\nformat = \"tr a-z A-Z\"\n").unwrap();
        let report = format_paths(&cfg, &dir, &["a.rs".into(), "b.md".into()]);
        assert_eq!(report.formatted, ["a.rs"]);
        assert!(report.warnings.is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("a.rs")).unwrap(), "ABC\n");
        assert_eq!(std::fs::read_to_string(dir.join("b.md")).unwrap(), "abc\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_template_runs_per_file() {
        let dir = temp("file");
        std::fs::write(dir.join("a.py"), "x\n").unwrap();
        let cfg = LanguageConfig::parse("[python]\nformat = \"echo y > {file}\"\n").unwrap();
        let report = format_paths(&cfg, &dir, &["a.py".into()]);
        assert_eq!(report.formatted, ["a.py"]);
        assert_eq!(std::fs::read_to_string(dir.join("a.py")).unwrap(), "y\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_formatter_warns_and_keeps_the_file() {
        let dir = temp("fail");
        std::fs::write(dir.join("a.rs"), "abc\n").unwrap();
        let cfg = LanguageConfig::parse("[rust]\nformat = \"cat >/dev/null; echo bad >&2; exit 2\"\n").unwrap();
        let report = format_paths(&cfg, &dir, &["a.rs".into()]);
        assert!(report.formatted.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("bad"));
        assert_eq!(std::fs::read_to_string(dir.join("a.rs")).unwrap(), "abc\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
