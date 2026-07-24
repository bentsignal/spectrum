use std::path::Path;

#[cfg(unix)]
use std::fs;

use crate::{RevisionError, RevisionResult};

/// Atomically publishes `source` at `destination` without replacing an existing
/// destination.
///
/// The paths must share a parent so that the rename and parent-directory sync
/// complete the fresh-destination publication transaction.
pub fn publish_noreplace(source: &Path, destination: &Path) -> RevisionResult<()> {
    if source.parent() != destination.parent() {
        return Err(RevisionError::Invalid(
            "no-replace publication requires source and destination to share a parent".to_owned(),
        ));
    }

    crate::live::rename_no_replace(source, destination)?;
    sync_parent(destination)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> RevisionResult<()> {
    fs::File::open(parent(path))?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> RevisionResult<()> {
    Ok(())
}

#[cfg(unix)]
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
