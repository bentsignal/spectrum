use std::{
    ffi::CString,
    fs::{self, File},
    io,
    os::{
        fd::{AsRawFd as _, FromRawFd as _},
        unix::fs::PermissionsExt as _,
    },
};

use super::{
    FileIdentity, PrivateDirectory, RevisionError, RevisionResult, SlotMutationPoint,
    maybe_inject_slot_hardlink, validated_identity,
};

const MUTATION_GATE: &str = ".published-mutation-gate";
const MUTATION_CANDIDATE: &str = ".published-mutation-candidate";

pub(crate) struct PrivateMutation<'a> {
    directory: &'a PrivateDirectory,
    gate: File,
    name: String,
    original_identity: FileIdentity,
    candidate: Option<File>,
    restored: bool,
}

impl<'a> PrivateMutation<'a> {
    pub(super) fn begin(
        directory: &'a PrivateDirectory,
        name: &str,
        descriptor: &File,
        identity: FileIdentity,
        point: SlotMutationPoint,
    ) -> RevisionResult<Option<Self>> {
        directory.validate(name, identity, true)?;
        let gate = open_gate(directory, true)?;
        ensure_missing(&gate, name)?;
        ensure_missing(&directory.descriptor, MUTATION_CANDIDATE)?;
        rename_between(&directory.descriptor, name, &gate, name)?;
        directory.sync()?;
        gate.sync_all()?;
        let mut guard = Self {
            directory,
            gate,
            name: name.into(),
            original_identity: identity,
            candidate: None,
            restored: false,
        };
        guard
            .gate
            .set_permissions(fs::Permissions::from_mode(0o0))?;
        validated_identity(descriptor, true)?;
        maybe_inject_slot_hardlink(descriptor, point)?;
        let candidate = match directory.clone_unnamed(descriptor) {
            Ok(candidate) => candidate,
            Err(_) => return Ok(None),
        };
        validate_unnamed(&candidate)?;
        guard.candidate = Some(candidate);
        super::super::maybe_publish_fault(super::super::PublishFault::SlotSealed)?;
        Ok(Some(guard))
    }

    pub(super) fn candidate(&self) -> &File {
        self.candidate
            .as_ref()
            .expect("successful private mutation has an anonymous candidate")
    }

    pub(super) fn finish(mut self, name: &str) -> RevisionResult<FileIdentity> {
        if name != self.name {
            return Err(RevisionError::Invalid(
                "private mutation guard name changed".into(),
            ));
        }
        let candidate = self
            .candidate
            .as_ref()
            .expect("successful private mutation has an anonymous candidate");
        let candidate_identity = validate_unnamed(candidate)?;
        link_unnamed(candidate, &self.directory.descriptor, MUTATION_CANDIDATE)?;
        self.directory.sync()?;
        self.directory
            .validate(MUTATION_CANDIDATE, candidate_identity, true)?;
        self.restore()?;
        if let Err(error) = self.directory.validate(name, self.original_identity, true) {
            let _ = self.directory.remove(MUTATION_CANDIDATE);
            let _ = self.directory.sync();
            return Err(error);
        }
        exchange(
            &self.directory.descriptor,
            name,
            &self.directory.descriptor,
            MUTATION_CANDIDATE,
        )?;
        self.directory.sync()?;
        if let Err(error) = self.directory.validate(name, candidate_identity, true) {
            let _ = exchange(
                &self.directory.descriptor,
                name,
                &self.directory.descriptor,
                MUTATION_CANDIDATE,
            );
            let _ = self.directory.sync();
            let _ = self.directory.remove(MUTATION_CANDIDATE);
            let _ = self.directory.sync();
            return Err(error);
        }
        self.directory.remove(MUTATION_CANDIDATE)?;
        self.directory.sync()?;
        Ok(candidate_identity)
    }

    fn restore(&mut self) -> RevisionResult<()> {
        if self.restored {
            return Ok(());
        }
        self.gate
            .set_permissions(fs::Permissions::from_mode(0o700))?;
        match openat(&self.gate, &self.name, false) {
            Ok(file) => {
                if validated_identity(&file, false)? != self.original_identity {
                    return Err(RevisionError::Invalid(
                        "mutation gate contains a replaced publication slot".into(),
                    ));
                }
                if openat(&self.directory.descriptor, &self.name, false).is_ok() {
                    return Err(RevisionError::Invalid(
                        "publication slot exists both inside and outside its mutation gate".into(),
                    ));
                }
                rename_between(
                    &self.gate,
                    &self.name,
                    &self.directory.descriptor,
                    &self.name,
                )?;
                self.gate.sync_all()?;
                self.directory.sync()?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.directory
                    .validate(&self.name, self.original_identity, false)?;
            }
            Err(error) => return Err(error.into()),
        }
        self.restored = true;
        Ok(())
    }
}

impl Drop for PrivateMutation<'_> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

pub(super) fn recover(directory: &PrivateDirectory) -> RevisionResult<()> {
    match open_gate(directory, false) {
        Ok(gate) => {
            let name = super::super::PUBLISH_MIRROR_FILE;
            match openat(&gate, name, false) {
                Ok(gated) => {
                    let identity = validated_identity(&gated, true)?;
                    if openat(&directory.descriptor, name, false).is_ok() {
                        return Err(RevisionError::Invalid(
                            "publication slot exists both inside and outside its mutation gate"
                                .into(),
                        ));
                    }
                    rename_between(&gate, name, &directory.descriptor, name)?;
                    gate.sync_all()?;
                    directory.sync()?;
                    directory.validate(name, identity, true)?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Err(RevisionError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    match directory.remove(MUTATION_CANDIDATE) {
        Ok(()) => directory.sync(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn validate_unnamed(descriptor: &File) -> RevisionResult<FileIdentity> {
    let metadata = descriptor.metadata()?;
    use std::os::unix::fs::MetadataExt as _;
    if !metadata.file_type().is_file() || metadata.nlink() != 0 {
        return Err(RevisionError::Invalid(
            "publication mutation candidate must remain an unnamed regular inode".into(),
        ));
    }
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

pub(super) fn link_unnamed(descriptor: &File, directory: &File, name: &str) -> io::Result<()> {
    let name = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))?;
    let linked = unsafe {
        libc::linkat(
            descriptor.as_raw_fd(),
            c"".as_ptr(),
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::AT_EMPTY_PATH,
        )
    };
    if linked == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if !matches!(
        error.raw_os_error(),
        Some(libc::EPERM) | Some(libc::ENOENT) | Some(libc::EINVAL)
    ) {
        return Err(error);
    }
    let descriptor_path =
        CString::new(format!("/proc/self/fd/{}", descriptor.as_raw_fd())).unwrap();
    let linked = unsafe {
        libc::linkat(
            libc::AT_FDCWD,
            descriptor_path.as_ptr(),
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::AT_SYMLINK_FOLLOW,
        )
    };
    if linked == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn open_gate(directory: &PrivateDirectory, create: bool) -> RevisionResult<File> {
    let name = CString::new(MUTATION_GATE).unwrap();
    if create {
        let created = unsafe {
            libc::mkdirat(
                directory.descriptor.as_raw_fd(),
                name.as_ptr(),
                libc::S_IRWXU,
            )
        };
        if created != 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error.into());
            }
        }
    }
    let path_descriptor = unsafe {
        libc::openat(
            directory.descriptor.as_raw_fd(),
            name.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if path_descriptor < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let path_descriptor = unsafe { File::from_raw_fd(path_descriptor) };
    validate_gate(&path_descriptor)?;
    restore_gate_permissions(&path_descriptor)?;
    let descriptor = unsafe {
        libc::openat(
            directory.descriptor.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let descriptor = unsafe { File::from_raw_fd(descriptor) };
    validate_gate(&descriptor)?;
    Ok(descriptor)
}

fn validate_gate(descriptor: &File) -> RevisionResult<()> {
    let metadata = descriptor.metadata()?;
    use std::os::unix::fs::MetadataExt as _;
    if !metadata.file_type().is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(RevisionError::Invalid(
            "publication mutation gate is not a private owned directory".into(),
        ));
    }
    Ok(())
}

fn restore_gate_permissions(descriptor: &File) -> RevisionResult<()> {
    let empty = c"";
    let restored = unsafe {
        libc::syscall(
            libc::SYS_fchmodat2,
            descriptor.as_raw_fd(),
            empty.as_ptr(),
            libc::S_IRWXU,
            libc::AT_EMPTY_PATH,
        )
    };
    if restored == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(libc::ENOSYS) && error.raw_os_error() != Some(libc::EINVAL) {
        return Err(error.into());
    }
    let descriptor_path = format!("/proc/self/fd/{}", descriptor.as_raw_fd());
    fs::set_permissions(descriptor_path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn openat(directory: &File, name: &str, write: bool) -> io::Result<File> {
    let name = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))?;
    let mut flags = libc::O_CLOEXEC | libc::O_NOFOLLOW;
    flags |= if write { libc::O_RDWR } else { libc::O_RDONLY };
    let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
    if descriptor < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }
}

fn ensure_missing(directory: &File, name: &str) -> RevisionResult<()> {
    match openat(directory, name, false) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => Err(RevisionError::Invalid(
            "mutation gate still contains a publication slot".into(),
        )),
    }
}

fn rename_between(
    source_dir: &File,
    source: &str,
    target_dir: &File,
    target: &str,
) -> io::Result<()> {
    fn name(value: &str) -> io::Result<CString> {
        CString::new(value)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))
    }
    let source = name(source)?;
    let target = name(target)?;
    let renamed = unsafe {
        libc::renameat(
            source_dir.as_raw_fd(),
            source.as_ptr(),
            target_dir.as_raw_fd(),
            target.as_ptr(),
        )
    };
    if renamed == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn exchange(source_dir: &File, source: &str, target_dir: &File, target: &str) -> io::Result<()> {
    fn name(value: &str) -> io::Result<CString> {
        CString::new(value)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))
    }
    let source = name(source)?;
    let target = name(target)?;
    let exchanged = unsafe {
        libc::renameat2(
            source_dir.as_raw_fd(),
            source.as_ptr(),
            target_dir.as_raw_fd(),
            target.as_ptr(),
            super::RENAME_EXCHANGE,
        )
    };
    if exchanged == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
