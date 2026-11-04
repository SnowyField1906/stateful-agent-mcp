//! Helpers shared by notes, handover tasks and turn metadata.

use crate::error::{AgentError, Result};
use crate::session::model::TurnKey;

use super::selector::{Sel, parse_item};

/// Single-line title; empty clears it.
pub(super) fn clean_title(raw: &str) -> Option<String> {
    let title = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

/// Item text without surrounding blank lines; must contain something other than whitespace.
pub(super) fn clean_text(raw: &str, arg: &str) -> Result<String> {
    let text = raw.trim_end().trim_start_matches(['\n', '\r']);
    if text.trim().is_empty() {
        return Err(AgentError::invalid(arg, "must not be empty"));
    }
    Ok(text.to_string())
}

/// Zero-based index of an item of the active turn, from `SS.TT.{kind}.NN` or a bare number.
pub(super) fn item_index(raw: &str, key: TurnKey, kind: &str, len: usize) -> Result<usize> {
    let id = raw.trim();
    let prefix = format!("{key}.{kind}.");
    let number = match id.strip_prefix(&prefix) {
        Some(rest) => rest,
        None if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => id,
        None => {
            let other = match parse_item(id) {
                Some(Sel::Note(k, _)) if kind == "note" => Some(k),
                Some(Sel::Task(k, _)) if kind == "task" => Some(k),
                _ => None,
            };
            return Err(match other {
                Some(k) => AgentError::ClosedTurn(k.to_string()),
                None => AgentError::invalid(
                    format!("{kind}_id"),
                    format!("`{id}` is not an ID of the active turn ({key}.{kind}.NN or NN)"),
                ),
            });
        }
    };
    let n: usize = number
        .parse()
        .map_err(|_| AgentError::invalid(format!("{kind}_id"), format!("`{id}` is not a number")))?;
    if n == 0 || n > len {
        return Err(AgentError::NotFound(format!("{kind} {id} (turn {key} has {len})")));
    }
    Ok(n - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes() {
        let key = TurnKey::new(2, 1);
        assert_eq!(item_index("02.01.note.02", key, "note", 3).unwrap(), 1);
        assert_eq!(item_index("3", key, "note", 3).unwrap(), 2);
        assert!(matches!(item_index("01.05.note.01", key, "note", 3), Err(AgentError::ClosedTurn(_))));
        assert!(item_index("4", key, "note", 3).is_err());
        assert!(item_index("0", key, "note", 3).is_err());
    }

    #[test]
    fn titles_and_text() {
        assert_eq!(clean_title("  a \n b  ").as_deref(), Some("a b"));
        assert_eq!(clean_title("   "), None);
        assert_eq!(clean_text("\n\n  x  \n\n", "n").unwrap(), "  x");
        assert!(clean_text(" \n ", "n").is_err());
    }
}
