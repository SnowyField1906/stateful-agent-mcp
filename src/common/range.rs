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
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{AgentError, Result};

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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Range {
        s.parse().unwrap()
    }

    #[test]
    fn parses_all_forms() {
        assert_eq!(parse("10..20"), Range::new(10, 20));
        assert_eq!(
            parse("10.."),
            Range {
                start: 10,
                end: None
            }
        );
        assert_eq!(parse("..20"), Range::new(1, 20));
        assert_eq!(parse(" 7 "), Range::new(7, 7));
        assert_eq!(
            parse("-50.."),
            Range {
                start: -50,
                end: None
            }
        );
        assert_eq!(
            parse(".."),
            Range {
                start: 1,
                end: None
            }
        );
    }

    #[test]
    fn rejects_bad_input() {
        for bad in ["0..5", "5..0", "9..3", "-5..9", "-5", "a..b", "1...3"] {
            assert!(bad.parse::<Range>().is_err(), "{bad}");
        }
    }

    #[test]
    fn displays_what_it_parses() {
        for text in ["10..20", "10..", "-50.."] {
            assert_eq!(parse(text).to_string(), text);
        }
    }

    #[test]
    fn windows() {
        assert_eq!(parse("3..5").forward(400, 2000, "lines").unwrap(), (2, 3));
        assert_eq!(parse("3..").forward(400, 2000, "lines").unwrap(), (2, 400));
        assert_eq!(
            parse("1..9999").forward(400, 2000, "lines").unwrap(),
            (0, 2000)
        );
        assert!(parse("-3..").forward(400, 2000, "lines").is_err());
        assert_eq!(parse("-3..").tail(), Some(3));
    }

    #[test]
    fn next_window_follows_the_shown_items() {
        assert_eq!(Range::next(200, 200).to_string(), "201..400");
    }
}
