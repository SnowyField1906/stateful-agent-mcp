//! Net effect between two states of the project as canonical file operations.

use std::collections::{BTreeMap, BTreeSet};

use crate::common::Result;

use super::diff::compute_hunks;
use super::patch::apply_hunks;
use super::{FileOp, PatchHunk, Tree};

fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Operations that turn `before` into `after`, preserving explicit `move_origins`
/// (`to_path -> pre_turn_from_path`) even when the moved file was also patched in the same turn.
/// Only paths present in `before` or `after` are considered. The result uses `Delete`, `Move`,
/// `Create` and `Patch` only, so it can be replayed with `write_files`.
///
/// Every modification to an existing file is represented as `FileOp::Patch` (never `FileOp::Create`).
/// When a file is moved and modified in the same turn, emits `Move { path, to_path }` followed by
/// `Patch { path: to_path, hunks }`.
pub fn net_ops(
    before: &Tree,
    after: &Tree,
    move_origins: &BTreeMap<String, String>,
) -> Vec<FileOp> {
    let paths: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let text = |tree: &'_ Tree, path: &str| tree.get(path).and_then(|v| v.clone());

    let mut moved_from: BTreeSet<String> = BTreeSet::new();
    let mut moved_to: BTreeSet<String> = BTreeSet::new();
    let mut moves: Vec<(String, String, String, String)> = Vec::new();

    for (to_path, from_path) in move_origins {
        if to_path == from_path || moved_from.contains(from_path) || moved_to.contains(to_path) {
            continue;
        }
        if has_move_cycle(move_origins, to_path) {
            continue;
        }
        if let (Some(old), Some(new)) = (text(before, from_path), text(after, to_path)) {
            moved_from.insert(from_path.clone());
            moved_to.insert(to_path.clone());
            moves.push((from_path.clone(), to_path.clone(), old, new));
        }
    }

    let mut deleted_candidates: Vec<(String, String)> = Vec::new();
    for path in &paths {
        if !moved_from.contains(*path)
            && let Some(old) = text(before, path)
            && (text(after, path).is_none() || moved_to.contains(*path))
        {
            deleted_candidates.push(((*path).clone(), old));
        }
    }
    let mut used_deleted = vec![false; deleted_candidates.len()];

    let mut plain_creates: Vec<FileOp> = Vec::new();
    for path in &paths {
        if moved_to.contains(*path) {
            continue;
        }
        let is_new_at_path = text(before, path).is_none() || moved_from.contains(*path);
        if !is_new_at_path {
            continue;
        }
        let Some(new_content) = text(after, path) else {
            continue;
        };
        let source = if new_content.is_empty() {
            None
        } else {
            deleted_candidates
                .iter()
                .enumerate()
                .filter(|(i, (_, old))| !used_deleted[*i] && *old == new_content)
                .min_by_key(|(_, (from, _))| base_name(from) != base_name(path))
                .map(|(i, _)| i)
        };
        match source {
            Some(i) => {
                used_deleted[i] = true;
                let (from_path, old_content) = &deleted_candidates[i];
                moved_from.insert(from_path.clone());
                moved_to.insert((*path).clone());
                moves.push((
                    from_path.clone(),
                    (*path).clone(),
                    old_content.clone(),
                    new_content,
                ));
            }
            None => plain_creates.push(FileOp::Create {
                path: (*path).clone(),
                content: new_content,
            }),
        }
    }

    let mut ops: Vec<FileOp> = Vec::new();
    for (i, (path, _)) in deleted_candidates.iter().enumerate() {
        if !used_deleted[i] {
            ops.push(FileOp::Delete { path: path.clone() });
        }
    }

    let ordered_moves = order_moves(&moves);
    for (from_path, to_path, _, _) in &ordered_moves {
        ops.push(FileOp::Move {
            path: from_path.clone(),
            to_path: to_path.clone(),
        });
    }

    ops.extend(plain_creates);

    let mut patches: BTreeMap<String, Vec<PatchHunk>> = BTreeMap::new();
    for path in &paths {
        if moved_from.contains(*path) || moved_to.contains(*path) {
            continue;
        }
        if let (Some(old), Some(new)) = (text(before, path), text(after, path))
            && old != new
        {
            let hunks = compute_hunks(&old, &new);
            if !hunks.is_empty() {
                patches.insert((*path).clone(), hunks);
            }
        }
    }
    for (_, to_path, old, new) in ordered_moves {
        if old != new {
            let hunks = compute_hunks(&old, &new);
            if !hunks.is_empty() {
                patches.insert(to_path, hunks);
            }
        }
    }
    for (path, hunks) in patches {
        ops.push(FileOp::Patch { path, hunks });
    }
    ops
}

fn has_move_cycle(move_origins: &BTreeMap<String, String>, start: &str) -> bool {
    let mut cur = start;
    for _ in 0..=move_origins.len() {
        let Some(prev) = move_origins.get(cur) else {
            return false;
        };
        if prev == start {
            return true;
        }
        cur = prev;
    }
    true
}

/// Order moves `(from, to, ..)` so that if move A's `to` is move B's `from`, A runs after B
/// (`B: to -> next` runs first to free `to`, then `A: from -> to`).
fn order_moves<T: Clone>(moves: &[(String, String, T, T)]) -> Vec<(String, String, T, T)> {
    let mut remaining: Vec<(String, String, T, T)> = moves.to_vec();
    let mut ordered = Vec::with_capacity(moves.len());
    while !remaining.is_empty() {
        let idx = remaining
            .iter()
            .position(|(_, to, _, _)| {
                !remaining
                    .iter()
                    .any(|(other_from, _, _, _)| other_from == to)
            })
            .unwrap_or(0);
        ordered.push(remaining.remove(idx));
    }
    ordered
}

/// Algebraically merge a sequence of [`FileOp`]s from one turn into canonical
/// `[Delete, Move, Create, Patch]` order.
pub fn merge_update_ops(ops: Vec<FileOp>) -> Result<Vec<FileOp>> {
    let mut deleted: BTreeSet<String> = BTreeSet::new();
    let mut moves: BTreeMap<String, String> = BTreeMap::new();
    let mut creates: BTreeMap<String, String> = BTreeMap::new();
    let mut patches: BTreeMap<String, Vec<PatchHunk>> = BTreeMap::new();

    for op in ops {
        match op {
            FileOp::Create { path, content } => {
                creates.insert(path, content);
            }
            FileOp::Patch { path, hunks } => {
                if let Some(content) = creates.get_mut(&path) {
                    let (patched, _) = apply_hunks(&path, content, &hunks)?;
                    *content = patched;
                } else {
                    let entry = patches.entry(path).or_default();
                    if entry.len() == 1
                        && hunks.len() == 1
                        && entry[0].new_text == hunks[0].old_text
                    {
                        entry[0].new_text = hunks[0].new_text.clone();
                        if entry[0].old_text == entry[0].new_text {
                            entry.clear();
                        }
                    } else {
                        entry.extend(hunks);
                    }
                }
            }
            FileOp::Move { path, to_path } => {
                if let Some(content) = creates.remove(&path) {
                    creates.insert(to_path, content);
                } else {
                    if let Some(hunks) = patches.remove(&path) {
                        patches.insert(to_path.clone(), hunks);
                    }
                    let orig = moves.remove(&path).unwrap_or(path);
                    if orig != to_path {
                        moves.insert(to_path, orig);
                    }
                }
            }
            FileOp::Delete { path } => {
                patches.remove(&path);
                if creates.remove(&path).is_some() {
                    // Created and deleted in the same turn: net no-op.
                } else if let Some(orig) = moves.remove(&path) {
                    deleted.insert(orig);
                } else {
                    deleted.insert(path);
                }
            }
            // Never recorded: the ledger holds the hunks these operations produced.
            FileOp::Recover { .. }
            | FileOp::ReplaceSymbol { .. }
            | FileOp::InsertBeforeSymbol { .. }
            | FileOp::InsertAfterSymbol { .. }
            | FileOp::RenameSymbol { .. } => {}
        }
    }

    let mut out = Vec::new();
    for path in deleted {
        out.push(FileOp::Delete { path });
    }
    let move_tuples: Vec<(String, String, (), ())> = moves
        .into_iter()
        .map(|(to, from)| (from, to, (), ()))
        .collect();
    for (from, to, _, _) in order_moves(&move_tuples) {
        out.push(FileOp::Move {
            path: from,
            to_path: to,
        });
    }
    for (path, content) in creates {
        out.push(FileOp::Create { path, content });
    }
    for (path, hunks) in patches {
        if !hunks.is_empty() {
            out.push(FileOp::Patch { path, hunks });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(items: &[(&str, Option<&str>)]) -> Tree {
        items
            .iter()
            .map(|(p, c)| (p.to_string(), c.map(str::to_string)))
            .collect()
    }

    fn net(before: &Tree, after: &Tree) -> Vec<FileOp> {
        net_ops(before, after, &BTreeMap::new())
    }

    #[test]
    fn detects_create_delete_move_patch() {
        let before = tree(&[
            ("old.rs", Some("fn a() {}\n")),
            ("gone.rs", Some("gone\n")),
            ("edit.rs", Some("1\n2\n3\n4\n5\n6\n7\n8\n")),
            ("new.rs", None),
            ("fresh.rs", None),
        ]);
        let after = tree(&[
            ("old.rs", None),
            ("gone.rs", None),
            ("edit.rs", Some("1\n2\n3\n4\nX\n6\n7\n8\n")),
            ("new.rs", Some("fn a() {}\n")),
            ("fresh.rs", Some("hello\n")),
        ]);
        let ops = net(&before, &after);
        assert!(ops.contains(&FileOp::Delete {
            path: "gone.rs".into()
        }));
        assert!(ops.contains(&FileOp::Move {
            path: "old.rs".into(),
            to_path: "new.rs".into()
        }));
        assert!(ops.contains(&FileOp::Create {
            path: "fresh.rs".into(),
            content: "hello\n".into()
        }));
        assert!(
            ops.iter()
                .any(|o| matches!(o, FileOp::Patch { path, .. } if path == "edit.rs"))
        );
    }

    #[test]
    fn identical_states_yield_nothing() {
        let t = tree(&[("a", Some("x")), ("b", None)]);
        assert!(net(&t, &t).is_empty());
    }

    #[test]
    fn existing_file_rewrites_and_empty_files_always_emit_patch() {
        let before = tree(&[("empty.rs", Some("")), ("rewrite.rs", Some("a\n"))]);
        let after = tree(&[
            ("empty.rs", Some("now populated\n")),
            ("rewrite.rs", Some("completely\ndifferent\ncontent\n")),
        ]);
        let ops = net(&before, &after);
        assert_eq!(ops.len(), 2);
        assert!(ops.iter().all(|op| matches!(op, FileOp::Patch { .. })));
    }

    #[test]
    fn move_and_patch_in_same_turn_emits_move_then_patch() {
        let before = tree(&[
            ("src/old.rs", Some("fn a() {\n    1\n}\n")),
            ("src/new.rs", None),
        ]);
        let after = tree(&[
            ("src/old.rs", None),
            ("src/new.rs", Some("fn a() {\n    2\n}\n")),
        ]);
        let mut moves = BTreeMap::new();
        moves.insert("src/new.rs".to_string(), "src/old.rs".to_string());

        let ops = net_ops(&before, &after, &moves);
        assert_eq!(ops.len(), 2);
        assert_eq!(
            ops[0],
            FileOp::Move {
                path: "src/old.rs".into(),
                to_path: "src/new.rs".into()
            }
        );
        assert!(matches!(&ops[1], FileOp::Patch { path, .. } if path == "src/new.rs"));
    }
}
