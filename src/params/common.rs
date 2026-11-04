//! Lenient deserializers shared by tool parameters. Agents often send numbers as
//! strings or a single string where a list is expected; these accept both.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};

use super::range::{MAX_ENTRIES, Range, opt_range};
use crate::error::Result;

#[derive(Deserialize)]
#[serde(untagged)]
enum NumOrStr {
    Num(u64),
    Str(String),
}

/// `Option<u64>` from a number or a numeric string (empty string means absent).
pub fn flex_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    use serde::de::Error;
    match Option::<NumOrStr>::deserialize(d)? {
        None => Ok(None),
        Some(NumOrStr::Num(n)) => Ok(Some(n)),
        Some(NumOrStr::Str(s)) => {
            let t = s.trim();
            if t.is_empty() {
                Ok(None)
            } else {
                t.parse::<u64>().map(Some).map_err(Error::custom)
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl From<OneOrMany> for Vec<String> {
    fn from(v: OneOrMany) -> Self {
        match v {
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }
}

/// `Vec<String>` from a string or a list of strings.
pub fn string_or_vec<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(OneOrMany::deserialize(d)?.into())
}

/// `Option<Vec<String>>` from a string or a list of strings.
pub fn opt_string_or_vec<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Vec<String>>, D::Error> {
    Ok(Option::<OneOrMany>::deserialize(d)?.map(Into::into))
}

/// Paging for listings.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PageParams {
    /// Entries to return, 1-based and inclusive: `1..200`, `201..`, `..50`, or `-50..` for the
    /// last 50. An open end uses the server's page size. Default: the first page.
    #[serde(default, deserialize_with = "opt_range")]
    #[schemars(with = "Option<String>")]
    pub entries: Option<Range>,
}

impl PageParams {
    /// Zero-based offset and length of the requested page.
    pub fn window(&self, default_len: usize) -> Result<(usize, usize)> {
        match self.entries {
            Some(range) => range.forward(default_len as u64, MAX_ENTRIES, "entries"),
            None => Ok((0, default_len.clamp(1, MAX_ENTRIES as usize))),
        }
    }

    /// Number of trailing entries when the page counts from the end (`-N..`).
    pub fn tail(&self) -> Option<usize> {
        self.entries
            .and_then(|range| range.tail())
            .map(|n| n.clamp(1, MAX_ENTRIES) as usize)
    }

    /// Like [`window`](Self::window), once the total is known: `-N..` selects the last N.
    pub fn window_in(&self, default_len: usize, total: usize) -> Result<(usize, usize)> {
        match self.tail() {
            Some(n) => Ok((total.saturating_sub(n), n)),
            None => self.window(default_len),
        }
    }
}
