//! Facts kept between calls, and a survey of the files below a scan target.
//!
//! A file's facts are reused while its size and modification time are what they were: the trust
//! rule of `workflow`'s `external` module (metadata matches, content unchanged). The store is in
//! memory and capped by entries; the least recently used file goes first.

use std::collections::HashMap;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use super::facts::{FileFacts, facts};
use crate::common::fs::walk::read_searchable;
use crate::common::{Result, Workspace};
use crate::engine::scan::{ScanTarget, files};
use crate::language::registry;

/// What the disk said about a file when its facts were read.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified_ns: u128,
}

impl Stamp {
    fn of(meta: &Metadata) -> Self {
        Self {
            size: meta.len(),
            modified_ns: meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_nanos()),
        }
    }
}

struct Entry {
    stamp: Stamp,
    /// Order of the last use, for eviction.
    used: u64,
    facts: Arc<FileFacts>,
}

#[derive(Default)]
struct Inner {
    clock: u64,
    entries: HashMap<PathBuf, Entry>,
}

/// The facts of the files read so far, by absolute path.
pub struct FactStore {
    capacity: usize,
    inner: Mutex<Inner>,
}

impl FactStore {
    /// A store that keeps the facts of at most `capacity` files.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            inner: Mutex::new(Inner::default()),
        }
    }

    /// How many files the store holds.
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The facts of the file at `abs`, whose shown path `label` names its language. They come from
    /// the store when the file is as it was, else the file is read and parsed. `None` when the
    /// language has no code intelligence or the file is unreadable, binary, oversized or cannot be
    /// parsed.
    pub fn facts(&self, abs: &Path, label: &str) -> Option<Arc<FileFacts>> {
        let syntax = registry::for_path(label)?;
        let stamp = Stamp::of(&std::fs::metadata(abs).ok()?);
        {
            let mut inner = self.lock();
            inner.clock += 1;
            let clock = inner.clock;
            if let Some(entry) = inner.entries.get_mut(abs)
                && entry.stamp == stamp
            {
                entry.used = clock;
                return Some(Arc::clone(&entry.facts));
            }
        }
        let mut read = facts(syntax, &read_searchable(abs)?)?;
        // A file of a test folder is test code whole: its functions carry no `mod tests` around them.
        if syntax.is_test_path(label) {
            for def in &mut read.defs {
                def.symbol.test = true;
            }
        }
        let read = Arc::new(read);
        let mut inner = self.lock();
        inner.clock += 1;
        let used = inner.clock;
        inner.entries.insert(
            abs.to_path_buf(),
            Entry {
                stamp,
                used,
                facts: Arc::clone(&read),
            },
        );
        while inner.entries.len() > self.capacity {
            let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(path, _)| path.clone())
            else {
                break;
            };
            inner.entries.remove(&oldest);
        }
        Some(read)
    }
}

/// The facts of the supported files below a scan target, in walk order.
pub struct Survey {
    /// Each file's shown path and facts.
    pub files: Vec<(String, Arc<FileFacts>)>,
    /// Files that could not be read or parsed.
    pub skipped: usize,
}

/// The facts of every supported file the target names or holds, through `store`.
pub fn survey(
    workspace: Option<&dyn Workspace>,
    target: &ScanTarget<'_>,
    store: &FactStore,
) -> Result<Survey> {
    let mut survey = Survey {
        files: Vec::new(),
        skipped: 0,
    };
    for (label, abs) in files(workspace, target)? {
        match store.facts(&abs, &label) {
            Some(facts) => survey.files.push((label, facts)),
            None => survey.skipped += 1,
        }
    }
    Ok(survey)
}
