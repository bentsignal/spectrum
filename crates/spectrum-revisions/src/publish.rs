use std::path::Path;

#[cfg(unix)]
use std::fs;

use crate::{RevisionError, RevisionResult};

/// Atomically publishes `source` at `destination` without replacing an existing
/// destination.
///
/// The paths must share a parent so that the rename and parent-directory sync
/// complete the fresh-destination publication transaction.
///
/// If the atomic rename succeeds but the parent sync fails, this returns
/// [`RevisionError::PublishedButNotSynced`]; in that state `destination` exists
/// and `source` no longer does.
pub fn publish_noreplace(source: &Path, destination: &Path) -> RevisionResult<()> {
    publish_noreplace_with_sync(source, destination, sync_parent)
}

fn publish_noreplace_with_sync(
    source: &Path,
    destination: &Path,
    sync: impl FnOnce(&Path) -> std::io::Result<()>,
) -> RevisionResult<()> {
    if source.parent() != destination.parent() {
        return Err(RevisionError::Invalid(
            "no-replace publication requires source and destination to share a parent".to_owned(),
        ));
    }

    crate::live::rename_no_replace(source, destination)?;
    sync(destination).map_err(|source| RevisionError::PublishedButNotSynced {
        destination: destination.to_owned(),
        source,
    })
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> std::io::Result<()> {
    fs::File::open(parent(path))?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(all(
    test,
    any(target_os = "linux", target_vendor = "apple", target_os = "windows")
))]
mod tests {
    use super::*;

    #[test]
    fn reports_that_publication_succeeded_when_parent_sync_fails() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("private");
        let destination = directory.path().join("published");
        std::fs::write(&source, b"candidate").unwrap();

        let error = publish_noreplace_with_sync(&source, &destination, |_| {
            Err(std::io::Error::other("injected sync failure"))
        })
        .unwrap_err();

        let RevisionError::PublishedButNotSynced {
            destination: reported,
            source: sync_error,
        } = error
        else {
            panic!("post-rename sync failure must have an explicit state");
        };
        assert_eq!(reported, destination);
        assert_eq!(sync_error.to_string(), "injected sync failure");
        assert!(!source.exists());
        assert_eq!(std::fs::read(destination).unwrap(), b"candidate");
    }
}
