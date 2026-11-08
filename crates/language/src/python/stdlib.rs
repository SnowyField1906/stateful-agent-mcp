//! The modules of the Python standard library, asked from the interpreter once per process.
//!
//! Some of them are compiled into the interpreter and have no file (`sys`, `builtins`), so the
//! folder of the standard library cannot tell them apart; the interpreter itself can.

use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

const SCRIPT: &str = "import sys\nnames = getattr(sys, 'stdlib_module_names', None)\nif names:\n    print('\\n'.join(sorted(names)))\n";

/// The names of the standard library modules, or `None` when no interpreter (3.10 or newer)
/// answers.
pub fn modules() -> Option<&'static HashSet<String>> {
    static MODULES: OnceLock<Option<HashSet<String>>> = OnceLock::new();
    MODULES.get_or_init(ask).as_ref()
}

fn ask() -> Option<HashSet<String>> {
    ["python3", "python"].iter().find_map(|exe| {
        let out = Command::new(exe)
            .args(["-I", "-S", "-c", SCRIPT])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let names: HashSet<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();
        (!names.is_empty()).then_some(names)
    })
}
