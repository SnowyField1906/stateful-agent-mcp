//! `START..END` ranges shared by line and listing parameters.
//!
//! A range is 1-based and inclusive. Accepted forms:
//!
//! | Form      | Meaning                                  |
//! |-----------|------------------------------------------|
//! | `10..20`  | positions 10 to 20                       |
//! | `10..`    | from 10, one default-sized window        |
//! | `..20`    | positions 1 to 20                        |
//! | `10`      | position 10 only                         |
//! | `-50..`   | the last 50 positions (reads only)       |

use std::fmt;
use std::num::NonZeroU64;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{AgentError, Result};

/// A 1-based line number in a text file. Guaranteed to be >= 1.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct LineNumber(NonZeroU64);

impl LineNumber {
    /// Line 1.
    pub const ONE: Self = Self(NonZeroU64::MIN);

    /// Create a 1-based line number if `val >= 1`.
    pub const fn new(val: u64) -> Option<Self> {
        match NonZeroU64::new(val) {
            Some(nz) => Some(Self(nz)),
            None => None,
        }
    }

    /// Convert a 0-based slice or line index to a 1-based line number.
    pub const fn from_zero_based(idx: usize) -> Self {
        match NonZeroU64::new(idx as u64 + 1) {
            Some(nz) => Self(nz),
            None => unreachable!(),
        }
    }

    /// The 1-based line number value.
    pub const fn get(&self) -> u64 {
        self.0.get()
    }

    /// The 0-based index of this line in a file's lines.
    pub const fn to_zero_based(&self) -> usize {
        (self.0.get() - 1) as usize
    }
}

impl fmt::Display for LineNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for LineNumber {
    type Err = AgentError;

    fn from_str(s: &str) -> Result<Self> {
        let val: u64 = s.trim().parse().map_err(|_| {
            AgentError::invalid("line", format!("expected positive integer, got `{s}`"))
        })?;
        Self::new(val)
            .ok_or_else(|| AgentError::invalid("line", "line numbers are 1-based and must be >= 1"))
    }
}

impl TryFrom<u64> for LineNumber {
    type Error = AgentError;

    fn try_from(val: u64) -> Result<Self> {
        Self::new(val)
            .ok_or_else(|| AgentError::invalid("line", "line numbers are 1-based and must be >= 1"))
    }
}

impl From<NonZeroU64> for LineNumber {
    fn from(nz: NonZeroU64) -> Self {
        Self(nz)
    }
}

impl From<LineNumber> for u64 {
    fn from(line: LineNumber) -> Self {
        line.get()
    }
}

/// Largest page of listing entries.
pub const MAX_ENTRIES: u64 = 5000;

/// A 1-based, inclusive `START..END` range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    /// First position. Negative counts back from the end (`-N..`).
    pub start: i64,
    /// Last position, or `None` for a default-sized window.
    pub end: Option<u64>,
}

impl Range {
    pub fn new(start: u64, end: u64) -> Self {
        Self {
            start: start as i64,
            end: Some(end),
        }
    }

    /// The range of `len` positions that follows the first `after` positions.
    pub fn next(after: usize, len: usize) -> Self {
        Self::new(after as u64 + 1, (after + len) as u64)
    }

    /// First position when the range counts from the start.
    pub fn first(&self) -> Option<u64> {
        u64::try_from(self.start).ok()
    }

    /// Number of trailing positions when the range counts from the end (`-N..`).
    pub fn tail(&self) -> Option<u64> {
        (self.start < 0).then(|| self.start.unsigned_abs())
    }

    /// Zero-based offset and length of the window this range selects. An open end uses
    /// `default_len`; the length is limited to `1..=max_len`.
    pub fn forward(&self, default_len: u64, max_len: u64, arg: &str) -> Result<(usize, usize)> {
        let start = self.first().ok_or_else(|| {
            AgentError::invalid(
                arg,
                format!("`{self}`; a negative start is not allowed here"),
            )
        })?;
        let len = self
            .end
            .map_or(default_len, |end| end - start + 1)
            .clamp(1, max_len);
        Ok((start as usize - 1, len as usize))
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.end {
            Some(end) => write!(f, "{}..{end}", self.start),
            None => write!(f, "{}..", self.start),
        }
    }
}

impl FromStr for Range {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let text = s.trim();
        let bad =
            || format!("`{text}`; expected `START..END`, `START..`, `..END`, `-N..` or one number");
        let (left, right) = match text.split_once("..") {
            Some((l, r)) => (l.trim(), Some(r.trim())),
            None => (text, None),
        };
        let start = if left.is_empty() {
            1
        } else {
            left.parse::<i64>().map_err(|_| bad())?
        };
        if start == 0 {
            return Err(format!("`{text}`; positions start at 1"));
        }
        let end = match right {
            None if start > 0 => Some(start as u64),
            None => return Err(bad()),
            Some("") => None,
            Some(r) => Some(r.parse::<u64>().map_err(|_| bad())?),
        };
        match end {
            Some(0) => return Err(format!("`{text}`; positions start at 1")),
            Some(_) if start < 0 => {
                return Err(format!("`{text}`; a negative start (last N) takes no end"));
            }
            Some(end) if end < start as u64 => {
                return Err(format!("`{text}`; the end is before the start"));
            }
            _ => {}
        }
        Ok(Self { start, end })
    }
}

impl Serialize for Range {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Raw {
    Num(i64),
    Str(String),
}

/// `Option<Range>` from a string (`10..20`) or a single number. An empty string means absent.
pub fn opt_range<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<Range>, D::Error> {
    use serde::de::Error;
    let text = match Option::<Raw>::deserialize(d)? {
        None => return Ok(None),
        Some(Raw::Num(n)) => n.to_string(),
        Some(Raw::Str(s)) => s,
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    text.parse().map(Some).map_err(Error::custom)
}
