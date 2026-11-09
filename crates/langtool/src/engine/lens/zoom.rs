//! `zoom`: the one word that says how much of an overview to show.
//!
//! The agent never sees a number of tokens. A zoom is a count of folder levels to open (its
//! `rings`) and a count of rows to spend (its `rows`), and the two grow together. Rows are ranked
//! without looking at the zoom, so a higher zoom only adds rows: it never reorders or removes one.

/// The longest row of an overview, in characters.
pub const ROW: usize = 120;

/// How much an overview shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Zoom {
    Low,
    #[default]
    Medium,
    High,
    Max,
}

impl Zoom {
    /// The zoom a word names: `low`, `medium`, `high`, `max`, their first letters or `1..4`, in any
    /// case. `None` for anything else.
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "low" | "l" | "1" => Some(Self::Low),
            "medium" | "med" | "m" | "2" => Some(Self::Medium),
            "high" | "h" | "3" => Some(Self::High),
            "max" | "4" => Some(Self::Max),
            _ => None,
        }
    }

    /// The zoom of an optional parameter, and a note when the word is not understood.
    pub fn from_param(word: Option<&str>) -> (Self, Option<String>) {
        let Some(word) = word.map(str::trim).filter(|word| !word.is_empty()) else {
            return (Self::default(), None);
        };
        match Self::parse(word) {
            Some(zoom) => (zoom, None),
            None => (
                Self::default(),
                Some(format!(
                    "note: zoom `{word}` is not low, medium, high or max; showing medium"
                )),
            ),
        }
    }

    /// How many folder levels, or hops around a focus, are opened.
    pub const fn rings(self) -> usize {
        match self {
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
            Self::Max => 4,
        }
    }

    /// How many rows a listing may spend.
    pub const fn rows(self) -> usize {
        match self {
            Self::Low => 24,
            Self::Medium => 48,
            Self::High => 96,
            Self::Max => 192,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }

    /// The next higher zoom; `None` at the top.
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Low => Some(Self::Medium),
            Self::Medium => Some(Self::High),
            Self::High => Some(Self::Max),
            Self::Max => None,
        }
    }
}

/// `text` cut to at most `max` characters, ending in `…` when something was left out.
pub fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    if max > 0 {
        cut.push('…');
    }
    cut
}

/// `row` cut to the longest row of an overview.
pub fn fit(row: &str) -> String {
    cut(row, ROW)
}

/// `text` with at most `max` lines; when lines are left out, the last line says how many. This is
/// the ceiling under the per-section caps of an overview, so no input can make it longer.
pub fn limit_lines(text: &str, max: usize) -> String {
    let total = text.lines().count();
    if total <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for line in text.lines().take(max.saturating_sub(1)) {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&format!(
        "… {} rows left out\n",
        total - max.saturating_sub(1)
    ));
    out
}
