use std::{
    ffi::CString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

use super::{
    FileIdentity, RevisionError, RevisionResult, SessionId, StorageStateId, sync_parent,
    temporary_path, validate_named_identity, validated_identity,
};

const FICLONE_IOCTL: libc::c_ulong = 0x4004_9409;

pub(super) struct StagedFile {
    _descriptor: File,
    path: PathBuf,
    identity: FileIdentity,
    published: bool,
}

impl StagedFile {
    pub(super) fn publish(mut self, destination: &Path) -> RevisionResult<()> {
        validate_named_identity(&self.path, self.identity, true)?;
        fs::rename(&self.path, destination)?;
        sync_parent(destination)?;
        if validated_identity(&super::open_nofollow(destination, false)?, false)? != self.identity {
            return Err(RevisionError::Invalid(
                "published path does not match its completed unnamed file".into(),
            ));
        }
        self.published = true;
        Ok(())
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
            let _ = sync_parent(&self.path);
        }
    }
}

pub(super) fn open_unnamed(directory: &Path) -> std::io::Result<File> {
    let directory = CString::new(directory.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory contains NUL"))?;
    let descriptor = unsafe {
        libc::open(
            directory.as_ptr(),
            libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
            0o600,
        )
    };
    if descriptor < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }
}

pub(super) fn clone_unnamed(source: &Path, directory: &Path) -> std::io::Result<File> {
    let source = super::open_nofollow(source, false)?;
    let candidate = open_unnamed(directory)?;
    let cloned = unsafe { libc::ioctl(candidate.as_raw_fd(), FICLONE_IOCTL, source.as_raw_fd()) };
    if cloned == 0 {
        Ok(candidate)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub(super) fn stage_unnamed(descriptor: File, destination: &Path) -> RevisionResult<StagedFile> {
    let path = temporary_path(destination);
    let temporary = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let linked = unsafe {
        libc::linkat(
            descriptor.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_FDCWD,
            temporary.as_ptr(),
            libc::AT_EMPTY_PATH,
        )
    };
    if linked != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    sync_parent(&path)?;
    let identity = validated_identity(&descriptor, true)?;
    validate_named_identity(&path, identity, true)?;
    Ok(StagedFile {
        _descriptor: descriptor,
        path,
        identity,
        published: false,
    })
}

pub(super) fn publish_unnamed(descriptor: File, destination: &Path) -> RevisionResult<()> {
    stage_unnamed(descriptor, destination)?.publish(destination)
}

fn marker_bytes(generation: u64, state_id: Option<StorageStateId>) -> Vec<u8> {
    let mut marker = generation.to_string();
    marker.push(':');
    match state_id {
        Some(state_id) => {
            for byte in state_id {
                use std::fmt::Write as _;
                write!(&mut marker, "{byte:02x}").expect("writing to a string cannot fail");
            }
        }
        None => marker.push('-'),
    }
    marker.into_bytes()
}

pub(super) fn ready_marker_matches(
    path: &Path,
    generation: u64,
    state_id: Option<StorageStateId>,
) -> bool {
    let Ok(mut marker) = super::open_nofollow(path, false) else {
        return false;
    };
    if validated_identity(&marker, true).is_err() {
        return false;
    }
    let mut bytes = Vec::new();
    marker.read_to_end(&mut bytes).is_ok() && bytes == marker_bytes(generation, state_id)
}

pub(super) fn write_ready_marker(
    path: &Path,
    generation: u64,
    state_id: Option<StorageStateId>,
) -> RevisionResult<()> {
    let temporary =
        path.with_file_name(format!(".published-mirror-ready-{}.tmp", SessionId::new()));
    let result = (|| -> RevisionResult<()> {
        let mut marker = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary)?;
        marker.write_all(&marker_bytes(generation, state_id))?;
        marker.sync_all()?;
        let identity = validated_identity(&marker, true)?;
        validate_named_identity(&temporary, identity, true)?;
        fs::rename(&temporary, path)?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        let _ = sync_parent(&temporary);
    }
    result
}
