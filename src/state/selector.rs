//! Selector syntax shared by the query tools and by `Recover`.
//!
//! - `01` session, `01.02` turn, `HEAD` active turn, `HEAD~N` the turn N before it
//! - `A..B` inclusive range of the above; `A..` runs to `HEAD`, `..B` starts at the first turn.
//!   A range written backwards (`01.05..01.02`) is reversed.
//! - `01.02.note.03`, `01.02.task.01`, `01.02.update.src/a.rs` single items

use crate::error::{AgentError, Result};
use crate::session::model::TurnKey;

use super::project::Project;

#[derive(Debug)]
pub(crate) enum Sel {
    /// Inclusive indexes into the flat turn list.
    Turns { lo: usize, hi: usize, reversed: bool },
    /// Zero-based item index within the turn.
    Note(TurnKey, usize),
    Task(TurnKey, usize),
    Update(TurnKey, String),
}

impl Project {
    pub(crate) fn parse_selector(&self, flat: &[TurnKey], raw: &str) -> Result<Sel> {
        let s = raw.trim();
        if s.is_empty() {
            return Err(AgentError::invalid("implicit_id", "empty selector"));
        }
        if let Some(item) = parse_item(s) {
            return Ok(item);
        }
        if let Some((from, to)) = s.split_once("..") {
            let lo = if from.trim().is_empty() { 0 } else { span(flat, from)?.0 };
            let hi = if to.trim().is_empty() { flat.len() - 1 } else { span(flat, to)?.1 };
            return Ok(if lo <= hi {
                Sel::Turns { lo, hi, reversed: false }
            } else {
                Sel::Turns { lo: hi, hi: lo, reversed: true }
            });
        }
        let (lo, hi) = span(flat, s)?;
        Ok(Sel::Turns { lo, hi, reversed: false })
    }
}

/// Parse `SS.TT.note.NN`, `SS.TT.task.NN` or `SS.TT.update.{path}`.
pub(crate) fn parse_item(s: &str) -> Option<Sel> {
    let mut parts = s.splitn(4, '.');
    let (a, b, kind, rest) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let key = TurnKey::new(a.parse().ok()?, b.parse().ok()?);
    match kind {
        "note" => Some(Sel::Note(key, rest.parse::<usize>().ok()?.checked_sub(1)?)),
        "task" => Some(Sel::Task(key, rest.parse::<usize>().ok()?.checked_sub(1)?)),
        "update" if !rest.is_empty() => Some(Sel::Update(key, rest.to_string())),
        _ => None,
    }
}

fn bad(raw: &str) -> AgentError {
    AgentError::invalid(
        "implicit_id",
        format!("`{}` is not a session (01), turn (01.02), range (01.02..01.05), HEAD[~N] or item ID", raw.trim()),
    )
}

/// First and last flat index covered by a session, turn or `HEAD` selector.
fn span(flat: &[TurnKey], raw: &str) -> Result<(usize, usize)> {
    let s = raw.trim();
    if let Some(rest) = s.strip_prefix("HEAD") {
        let back = match rest {
            "" => 0,
            "~" => 1,
            r => r
                .strip_prefix('~')
                .and_then(|n| n.parse::<usize>().ok())
                .ok_or_else(|| bad(raw))?,
        };
        let idx = (flat.len() - 1).checked_sub(back).ok_or_else(|| {
            AgentError::NotFound(format!("{s}: history has only {} turns", flat.len()))
        })?;
        return Ok((idx, idx));
    }
    if let Some(key) = TurnKey::parse(s) {
        let idx = flat
            .iter()
            .position(|k| *k == key)
            .ok_or_else(|| AgentError::NotFound(format!("turn {key}")))?;
        return Ok((idx, idx));
    }
    if let Ok(session) = s.parse::<u32>() {
        let first = flat.iter().position(|k| k.session == session);
        let last = flat.iter().rposition(|k| k.session == session);
        return match (first, last) {
            (Some(f), Some(l)) => Ok((f, l)),
            _ => Err(AgentError::NotFound(format!("session {session:02}"))),
        };
    }
    Err(bad(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items() {
        assert!(matches!(parse_item("01.02.note.03"), Some(Sel::Note(k, 2)) if k == TurnKey::new(1, 2)));
        assert!(matches!(parse_item("1.2.update.a.b/c.rs"), Some(Sel::Update(_, p)) if p == "a.b/c.rs"));
        assert!(parse_item("01.02").is_none());
        assert!(parse_item("01.02.note.00").is_none());
    }

    #[test]
    fn spans() {
        let flat = [TurnKey::new(1, 1), TurnKey::new(1, 2), TurnKey::new(2, 1)];
        assert_eq!(span(&flat, "01").unwrap(), (0, 1));
        assert_eq!(span(&flat, "02.01").unwrap(), (2, 2));
        assert_eq!(span(&flat, "HEAD~1").unwrap(), (1, 1));
        assert!(span(&flat, "HEAD~3").is_err());
        assert!(span(&flat, "03").is_err());
    }
}
