use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{RevisionError, RevisionResult};

/// Atomically moves a private file into a fresh destination without replacement.
///
/// The source and destination must share a directory so publication cannot cross
/// filesystem boundaries. Existing files, directories, and symlinks are never replaced.
pub fn publish_noreplace(source: &Path, destination: &Path) -> RevisionResult<()> {
    if source.parent() != destination.parent() {
        return Err(RevisionError::Invalid(
            "atomic publication requires a same-directory temporary file".into(),
        ));
    }
    rename_noreplace(source, destination)?;
    sync_parent(destination)
}

#[cfg(target_os = "linux")]
fn rename_noreplace(source: &Path, destination: &Path) -> RevisionResult<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| RevisionError::Invalid("source path contains a NUL byte".into()))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| RevisionError::Invalid("destination path contains a NUL byte".into()))?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}

#[cfg(target_os = "macos")]
fn rename_noreplace(source: &Path, destination: &Path) -> RevisionResult<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| RevisionError::Invalid("source path contains a NUL byte".into()))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| RevisionError::Invalid("destination path contains a NUL byte".into()))?;
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}

#[cfg(windows)]
fn rename_noreplace(source: &Path, destination: &Path) -> RevisionResult<()> {
    use std::os::windows::ffi::OsStrExt;

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileW(source.as_ptr(), destination.as_ptr())
    };
    if result != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn rename_noreplace(source: &Path, destination: &Path) -> RevisionResult<()> {
    fs::hard_link(source, destination)?;
    fs::remove_file(source)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> RevisionResult<()> {
    fs::File::open(parent(path))?.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn sync_parent(path: &Path) -> RevisionResult<()> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(parent(path))?
        .sync_all()?;
    Ok(())
}

fn parent(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_owned()
}
