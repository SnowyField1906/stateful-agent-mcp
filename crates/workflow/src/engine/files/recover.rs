//! `Recover`: restores files to their content at a past turn.

use std::collections::BTreeSet;

use crate::common::pathspec::{matches_pathspec, normalize_rel};
use crate::common::{AgentError, Result};
use crate::engine::Project;
use crate::engine::ops::overlay::Overlay;
use crate::engine::selector::Sel;
use crate::engine::session::model::TurnKey;

use super::op::is_dir_view;

/// The turns a recovery reads from: files go back to their content before turn index `target`,
/// or before `fallback` when they did not exist then and are absent now.
#[derive(Clone, Copy)]
struct Span<'a> {
    flat: &'a [TurnKey],
    target: usize,
    fallback: usize,
}

impl Project {
    /// Restore files to their content at a past turn.
    /// - update ID (`01.02.update.src/a.rs`): restores that file (optionally into `to_path`)
    /// - turn, session or range with `to_path`: restores that file or directory prefix
    /// - turn, session or range without `to_path`: restores every file those turns touched
    pub(super) fn recover(
        &self,
        ov: &mut Overlay,
        flat: &[TurnKey],
        id: &str,
        to_path: Option<&str>,
    ) -> Result<String> {
        match self.parse_selector(flat, id)? {
            Sel::Update(key, path) => {
                let idx = flat
                    .iter()
                    .position(|k| *k == key)
                    .ok_or_else(|| AgentError::NotFound(format!("turn {key}")))?;
                let path = normalize_rel(&path, &self.root)?;
                if !self.update_touches(key, &path) {
                    return Err(AgentError::NotFound(id.trim().to_string()));
                }
                let target = to_path.unwrap_or(&path);
                if is_dir_view(ov, target)? {
                    return Err(AgentError::invalid(target, "is a directory"));
                }
                let span = Span {
                    flat,
                    target: idx + 1,
                    fallback: idx,
                };
                let content = self.restored_content(ov, span, &path)?;
                let summary = recovered(id, target, content.is_some());
                ov.set(target, content)?;
                Ok(summary)
            }
            Sel::Turns { lo, hi, reversed } => {
                let span = Span {
                    flat,
                    target: if reversed { lo + 1 } else { hi + 1 },
                    fallback: lo,
                };
                if let Some(spec) = to_path {
                    return self.recover_pathspec(ov, span, id, spec);
                }
                let paths: BTreeSet<String> = flat[lo..=hi]
                    .iter()
                    .flat_map(|key| self.touched_paths(*key))
                    .collect();
                self.restore_paths(ov, span, &paths)?;
                Ok(format!("Recover {id} ({} files)", paths.len()))
            }
            _ => Err(AgentError::invalid(
                "id",
                "expected an update, turn, session or range ID",
            )),
        }
    }

    /// Restore the file or directory prefix `spec` as of the span.
    fn recover_pathspec(
        &self,
        ov: &mut Overlay,
        span: Span<'_>,
        id: &str,
        spec: &str,
    ) -> Result<String> {
        let mut candidates: BTreeSet<String> = span
            .flat
            .iter()
            .flat_map(|key| self.touched_paths(*key))
            .filter(|path| matches_pathspec(path, spec))
            .collect();
        let spec_is_dir = is_dir_view(ov, spec)?;
        if spec_is_dir {
            candidates.extend(ov.list_files(spec)?);
        }
        let is_prefix_match = spec_is_dir
            || candidates
                .iter()
                .any(|p| p != spec && matches_pathspec(p, spec));
        if is_prefix_match {
            self.restore_paths(ov, span, &candidates)?;
            return Ok(format!(
                "Recover {id} → {spec} ({} files)",
                candidates.len()
            ));
        }

        let content = self.restored_content(ov, span, spec)?;
        if content.is_none() && ov.get(spec)?.is_none() && !candidates.contains(spec) {
            return Err(AgentError::NotFound(format!("{spec} at {id}")));
        }
        let summary = recovered(id, spec, content.is_some());
        ov.set(spec, content)?;
        Ok(summary)
    }

    /// Put every path back to its content as of the span.
    fn restore_paths(
        &self,
        ov: &mut Overlay,
        span: Span<'_>,
        paths: &BTreeSet<String>,
    ) -> Result<()> {
        for path in paths {
            let content = self.restored_content(ov, span, path)?;
            ov.set(path, content)?;
        }
        Ok(())
    }

    /// The content `path` gets back: what it held before `span.target`. A file that did not exist
    /// then and is absent now (deleted by the recovered turns) comes back as it was before
    /// `span.fallback`.
    fn restored_content(
        &self,
        ov: &mut Overlay,
        span: Span<'_>,
        path: &str,
    ) -> Result<Option<String>> {
        let content = self.content_before(span.flat, span.target, path)?;
        if content.is_none() && ov.get(path)?.is_none() {
            return self.content_before(span.flat, span.fallback, path);
        }
        Ok(content)
    }

    /// True when the turn's recorded updates or deleted files include `path`.
    fn update_touches(&self, key: TurnKey, path: &str) -> bool {
        self.turn(key)
            .is_some_and(|t| t.updates.iter().any(|op| op.touches_path(path)))
            || self
                .deleted
                .get(&key)
                .is_some_and(|files| files.contains_key(path))
    }
}

/// The summary of recovering one file; a file the span did not hold is reported as deleted.
fn recovered(id: &str, target: &str, existed: bool) -> String {
    format!(
        "Recover {id} → {target}{}",
        if existed { "" } else { " (deleted)" }
    )
}
