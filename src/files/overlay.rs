//! A virtual view of the project's files used to apply a batch of operations in memory.
//! Nothing touches the disk until [`Overlay::commit`], which makes batches atomic with
//! respect to operation errors.

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::error::{AgentError, Result};
use crate::files::path::is_protected_name;
use crate::session::store::atomic_write;

const MAX_DIR_FILES: usize = 10_000;

/// A touched file: its content when first seen and its content now.
#[derive(Clone, Debug)]
pub struct Entry {
    pub baseline: Option<String>,
    pub current: Option<String>,
}

pub struct Overlay {
    root: PathBuf,
    entries: BTreeMap<String, Entry>,
}

/// Read a text file; `None` when it does not exist. Non-UTF-8 content is an error because
/// history and patching are text-based.
pub fn read_text_opt(root: &Path, rel: &str) -> Result<Option<String>> {
    let path = root.join(rel);
    match std::fs::metadata(&path) {
        Ok(meta) if meta.is_dir() => Err(AgentError::invalid(rel, "is a directory")),
        Ok(_) => {
            let bytes = std::fs::read(&path).map_err(|e| AgentError::io_at(&path, e))?;
            String::from_utf8(bytes).map(Some).map_err(|_| {
                AgentError::invalid(rel, "file is not valid UTF-8 text; binary files are not supported")
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AgentError::io_at(&path, e)),
    }
}

impl Overlay {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            entries: BTreeMap::new(),
        }
    }

    /// Disk path for `rel`, rejecting symlinks anywhere along it.
    fn disk_path(&self, rel: &str) -> Result<PathBuf> {
        let mut path = self.root.clone();
        for part in rel.split('/') {
            path.push(part);
            if let Ok(meta) = std::fs::symlink_metadata(&path)
                && meta.file_type().is_symlink()
            {
                return Err(AgentError::ProtectedPath(format!(
                    "{rel}: write operations do not follow symlinks"
                )));
            }
        }
        Ok(path)
    }

    fn ensure_loaded(&mut self, rel: &str) -> Result<()> {
        if !self.entries.contains_key(rel) {
            self.disk_path(rel)?;
            let content = read_text_opt(&self.root, rel)?;
            self.entries.insert(
                rel.to_string(),
                Entry {
                    baseline: content.clone(),
                    current: content,
                },
            );
        }
        Ok(())
    }

    /// Current content of `rel` (`None` if absent).
    pub fn get(&mut self, rel: &str) -> Result<Option<String>> {
        self.ensure_loaded(rel)?;
        Ok(self.entries[rel].current.clone())
    }

    pub fn set(&mut self, rel: &str, value: Option<String>) -> Result<()> {
        self.ensure_loaded(rel)?;
        if let Some(entry) = self.entries.get_mut(rel) {
            entry.current = value;
        }
        Ok(())
    }

    pub fn is_dir(&self, rel: &str) -> bool {
        self.root.join(rel).is_dir()
    }

    /// Files under directory `rel`: those on disk plus created ones, minus deleted ones.
    pub fn list_files(&self, rel: &str) -> Result<Vec<String>> {
        let mut files = BTreeSet::new();
        walk_files(&self.root.join(rel), rel, &mut files)?;
        let prefix = format!("{rel}/");
        for (path, entry) in &self.entries {
            if path.starts_with(&prefix) {
                if entry.current.is_some() {
                    files.insert(path.clone());
                } else {
                    files.remove(path);
                }
            }
        }
        Ok(files.into_iter().collect())
    }

    /// All loaded entries in this overlay.
    pub fn entries(&self) -> &BTreeMap<String, Entry> {
        &self.entries
    }

    /// Entries whose content differs from the baseline.
    pub fn changed(&self) -> impl Iterator<Item = (&String, &Entry)> {
        self.entries.iter().filter(|(_, e)| e.baseline != e.current)
    }

    /// Write all changes to disk: removals first, then writes.
    pub fn commit(&self) -> Result<()> {
        for (rel, _) in self.changed().filter(|(_, e)| e.current.is_none()) {
            let path = self.disk_path(rel)?;
            std::fs::remove_file(&path).map_err(|e| AgentError::io_at(&path, e))?;
            self.prune_empty_parents(&path);
        }
        for (rel, entry) in self.changed() {
            let Some(content) = &entry.current else { continue };
            let path = self.disk_path(rel)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AgentError::io_at(parent, e))?;
            }
            atomic_write(&path, content.as_bytes())?;
        }
        Ok(())
    }

    /// Undo a failed [`Overlay::commit`]: put every changed file back to its baseline. Best
    /// effort, so that the commit error is the one reported.
    pub fn revert(&self) {
        for (rel, entry) in self.changed() {
            let Ok(path) = self.disk_path(rel) else { continue };
            match &entry.baseline {
                Some(text) => {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = atomic_write(&path, text.as_bytes());
                }
                None => {
                    if std::fs::remove_file(&path).is_ok() {
                        self.prune_empty_parents(&path);
                    }
                }
            }
        }
    }

    /// Remove directories left empty by a deletion, up to (not including) the project root.
    fn prune_empty_parents(&self, file: &Path) {
        let mut dir = file.parent();
        while let Some(d) = dir {
            if d == self.root || !d.starts_with(&self.root) || std::fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
    }
}

fn walk_files(dir: &Path, rel: &str, out: &mut BTreeSet<String>) -> Result<()> {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        // A missing path and a regular file both mean "no files below this directory".
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => return Ok(()),
        Err(e) => return Err(AgentError::io_at(dir, e)),
    };
    for entry in read {
        let entry = entry.map_err(|e| AgentError::io_at(dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_protected_name(&name) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else { continue };
        let child = format!("{rel}/{name}");
        if file_type.is_dir() {
            walk_files(&entry.path(), &child, out)?;
        } else if file_type.is_file() {
            out.insert(child);
        }
        if out.len() > MAX_DIR_FILES {
            return Err(AgentError::invalid(rel, format!("directory has more than {MAX_DIR_FILES} files")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revert_undoes_the_files_written_before_a_failed_commit() {
        let dir = std::env::temp_dir().join(format!("agent-mcp-overlay-revert-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // A directory named like the temporary file makes the write of `x.txt` fail after
        // `a.txt` was written and `b.txt` was removed.
        std::fs::create_dir_all(dir.join("x.txt.tmp")).unwrap();
        std::fs::write(dir.join("a.txt"), "old\n").unwrap();
        std::fs::write(dir.join("b.txt"), "kept\n").unwrap();

        let mut overlay = Overlay::new(dir.clone());
        overlay.set("a.txt", Some("new\n".into())).unwrap();
        overlay.set("b.txt", None).unwrap();
        overlay.set("x.txt", Some("x\n".into())).unwrap();
        assert!(overlay.commit().is_err());

        overlay.revert();

        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "old\n");
        assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "kept\n");
        assert!(!dir.join("x.txt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
