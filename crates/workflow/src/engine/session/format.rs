//! The format of `.AGENTS/sessions/{SS}.md`, shared by `serialize` and `parse`, which are a
//! lossless round trip.
//!
//! ```text
//! ## TURN 01.01 — optional title
//!
//! ### Notes
//!
//! 1. [REF 01.01.note.01] Note text
//!    Continuation lines are indented to the item text.
//!
//! ### Updates
//!
//! <details>
//! <summary>Patch <code>src/a.rs</code> (+3 -1)</summary>
//!
//! ```diff
//! -old line
//! +new line
//! ...
//! ```
//!
//! </details>
//!
//! (one `<details>` per file operation; hunks of one patch are separated by `...`)
//!
//! ### Handover Tasks
//!
//! 1. [ROLLED-OVER 01.01.task.02] [USER-EXPLICIT] Task text
//! ```

/// Start of a turn header line: `## TURN 01.02 — title`.
pub(super) const TURN_PREFIX: &str = "## TURN ";

/// Start of the heading of a section within a turn.
pub(super) const SECTION_PREFIX: &str = "### ";

pub(super) const FENCE: &str = "```";

/// Tags that open the text of a note or a task. Item text that starts with one of the tags of
/// its kind is escaped with a leading backslash, so that it is not read back as metadata.
pub(super) const REF_TAG: &str = "[REF ";
pub(super) const ROLLED_OVER_TAG: &str = "[ROLLED-OVER";
pub(super) const USER_EXPLICIT_TAG: &str = "[USER-EXPLICIT]";
pub(super) const NOTE_TAGS: &[&str] = &[REF_TAG];
pub(super) const TASK_TAGS: &[&str] = &[ROLLED_OVER_TAG, USER_EXPLICIT_TAG];

/// The sections of a turn.
#[derive(Clone, Copy)]
pub(super) enum Section {
    Notes,
    Updates,
    Tasks,
}

impl Section {
    const ALL: [Self; 3] = [Self::Notes, Self::Updates, Self::Tasks];

    pub(super) const fn heading(self) -> &'static str {
        match self {
            Self::Notes => "### Notes",
            Self::Updates => "### Updates",
            Self::Tasks => "### Handover Tasks",
        }
    }

    /// The section that `heading` opens; `None` for any other line.
    pub(super) fn of(heading: &str) -> Option<Self> {
        let heading = heading.trim();
        Self::ALL.into_iter().find(|s| s.heading() == heading)
    }
}

pub(super) fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(super) fn html_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// True when `s` begins with one or more `\` characters followed immediately by `[`.
fn starts_with_escaped_bracket(s: &str) -> bool {
    let slashes = s.bytes().take_while(|&b| b == b'\\').count();
    slashes > 0 && s.as_bytes().get(slashes) == Some(&b'[')
}

/// Prefix a backslash when `content` could be mistaken for a tag or for an escape.
pub(super) fn protect(content: &str, tags: &[&str]) -> String {
    if starts_with_escaped_bracket(content) || tags.iter().any(|t| content.starts_with(t)) {
        format!("\\{content}")
    } else {
        content.to_string()
    }
}

/// Inverse of [`protect`].
pub(super) fn unprotect(text: &str) -> &str {
    if starts_with_escaped_bracket(text) {
        &text[1..]
    } else {
        text
    }
}
