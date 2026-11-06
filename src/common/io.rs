//! Atomic file writes.

use std::path::Path;

use super::{AgentError, Result};

/// Write `data` to `path` atomically (temporary file + rename).
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    std::fs::write(&tmp, data).map_err(|e| AgentError::io_at(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| AgentError::io_at(path, e))
}
