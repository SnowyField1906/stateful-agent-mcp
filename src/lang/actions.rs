//! Planning and reporting of project actions (`lint`, `build`, `test`, `install`, `remove`).

use std::time::Duration;

use super::config::{ARGS_PLACEHOLDER, Action, LanguageTooling};
use super::exec::Captured;

/// Upper bound for one action.
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(600);

/// Command to execute and an optional hint to show after execution.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub command: String,
    pub hint: Option<String>,
}

/// Combine the configured `command` with the caller's `args`.
pub fn plan(language: &str, action: Action, command: &str, args: Option<&str>) -> Plan {
    let label = format!("{language} {}", action.name());
    match (LanguageTooling::accepts_args(command), args.map(str::trim)) {
        (true, Some(args)) if !args.is_empty() => Plan {
            command: command.replace(ARGS_PLACEHOLDER, args),
            hint: None,
        },
        (true, _) => Plan {
            command: command.replace(ARGS_PLACEHOLDER, "").trim().to_string(),
            hint: Some(format!(
                "hint: `{label}` accepts arguments (`{command}`); try passing `--help` in `args` for more information."
            )),
        },
        (false, Some(args)) if !args.is_empty() => Plan {
            command: command.to_string(),
            hint: Some(format!(
                "hint: `{label}` takes no arguments (`{command}`); passed `args` was ignored."
            )),
        },
        (false, _) => Plan {
            command: command.to_string(),
            hint: None,
        },
    }
}

/// Remove ANSI escape sequences, normalize line endings and collapse `\r` progress updates.
pub fn clean_output(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            plain.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for n in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(n) = chars.next() {
                    if n == '\u{7}' {
                        break;
                    }
                    if n == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    plain
        .replace("\r\n", "\n")
        .split('\n')
        .map(|line| line.rsplit('\r').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Text returned to the agent: command, exit status, then stdout and stderr.
pub fn report(command: &str, out: &Captured) -> String {
    let secs = out.elapsed.as_secs_f32();
    let status = if out.timed_out {
        format!("timed out after {secs:.1}s and was killed")
    } else {
        match out.status {
            Some(code) => format!("exit {code} in {secs:.1}s"),
            None => format!("terminated by a signal after {secs:.1}s"),
        }
    };
    let stdout = clean_output(&String::from_utf8_lossy(&out.stdout));
    let stderr = clean_output(&String::from_utf8_lossy(&out.stderr));
    let mut text = format!("$ {command}\n{status}\n");
    if !stdout.trim().is_empty() {
        text.push_str(stdout.trim_end());
        text.push('\n');
    }
    if !stderr.trim().is_empty() {
        if !stdout.trim().is_empty() {
            text.push_str("--- stderr ---\n");
        }
        text.push_str(stderr.trim_end());
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_are_substituted_or_hinted() {
        assert_eq!(
            plan("rust", Action::Test, "cargo test {args}", Some(" -q ")),
            Plan {
                command: "cargo test -q".into(),
                hint: None,
            }
        );
        let no_args = plan("rust", Action::Test, "cargo test {args}", None);
        assert_eq!(no_args.command, "cargo test");
        assert!(
            no_args
                .hint
                .as_deref()
                .is_some_and(|h| h.contains("--help"))
        );

        let empty_args = plan("rust", Action::Test, "cargo test {args}", Some(""));
        assert_eq!(empty_args.command, "cargo test");
        assert!(
            empty_args
                .hint
                .as_deref()
                .is_some_and(|h| h.contains("--help"))
        );

        let extra_args = plan("rust", Action::Build, "cargo build", Some("--release"));
        assert_eq!(extra_args.command, "cargo build");
        assert!(
            extra_args
                .hint
                .as_deref()
                .is_some_and(|h| h.contains("takes no arguments"))
        );

        assert_eq!(
            plan("rust", Action::Build, "cargo build", None),
            Plan {
                command: "cargo build".into(),
                hint: None,
            }
        );
        assert_eq!(
            plan("rust", Action::Build, "cargo build", Some("")),
            Plan {
                command: "cargo build".into(),
                hint: None,
            }
        );
    }

    #[test]
    fn cleans_ansi_and_progress_lines() {
        assert_eq!(
            clean_output("\u{1b}[31merror\u{1b}[0m: x\r\nok"),
            "error: x\nok"
        );
        assert_eq!(clean_output("10%\r50%\r100%\ndone"), "100%\ndone");
    }
}
