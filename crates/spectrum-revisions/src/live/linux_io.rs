#[cfg(test)]
use std::cell::RefCell;
use std::{
    ffi::CString,
    fs::{self, File},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::PermissionsExt},
    },
    path::{Path, PathBuf},
};

use super::{
    FileIdentity, PublishStats, PublishStrategy, PublishTimings, RevisionError, RevisionResult,
    RevisionStore, SessionId, StorageStateId, sync_parent, temporary_path, validate_named_identity,
    validated_identity,
};

const RENAME_EXCHANGE: libc::c_uint = 2;
mod markers;
mod mutation;
mod publication;
pub(super) use markers::{
    WorkingRecoveryInstall, publication_marker_bytes_match, publication_marker_matches,
    read_publication_marker, read_working_recovery_marker, working_recovery_marker_bytes_match,
    working_recovery_poison_present, write_publication_marker, write_ready_marker,
    write_working_recovery_marker,
};
#[cfg(test)]
pub(super) use markers::{ready_marker_matches, working_recovery_marker_matches};
pub(super) use publication::{
    PublishBase, recover_full_copy, validate_publish_base, write_full_copy_intent,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SlotMutationPoint {
    CandidateDelta,
    PermissionRepair,
    BulkCatchUp,
}

#[cfg(test)]
thread_local! {
    static SLOT_HARDLINK_RACE: RefCell<Option<(SlotMutationPoint, PathBuf)>> =
        const { RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn set_slot_hardlink_race(point: SlotMutationPoint, alias: PathBuf) {
    SLOT_HARDLINK_RACE.with(|hook| hook.replace(Some((point, alias))));
}

#[cfg(test)]
pub(super) fn clear_slot_hardlink_race() {
    SLOT_HARDLINK_RACE.with(|hook| hook.replace(None));
}

#[cfg(test)]
fn maybe_inject_canonical_hardlink(destination: &Path) -> RevisionResult<()> {
    let alias = super::PUBLISH_HARDLINK_ALIAS.with(|alias| alias.borrow_mut().take());
    if let Some(alias) = alias {
        fs::hard_link(destination, alias)?;
    }
    Ok(())
}

#[cfg(not(test))]
fn maybe_inject_canonical_hardlink(_destination: &Path) -> RevisionResult<()> {
    Ok(())
}

pub(super) enum ExchangeOutcome {
    CanonicalLinked,
    Exchanged { old_slot_private: bool },
}

pub(super) struct PrivateDirectory {
    descriptor: File,
    #[cfg(test)]
    path: PathBuf,
}

impl PrivateDirectory {
    pub(super) fn open(path: &Path) -> RevisionResult<Self> {
        #[cfg(test)]
        let path_buf = path.to_path_buf();
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory contains NUL"))?;
        let descriptor = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if descriptor < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let descriptor = unsafe { File::from_raw_fd(descriptor) };
        let metadata = descriptor.metadata()?;
        use std::os::unix::fs::MetadataExt as _;
        if !metadata.file_type().is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(RevisionError::Invalid(
                "live cache directory must be owned by this user and inaccessible to group/other"
                    .into(),
            ));
        }
        let directory = Self {
            descriptor,
            #[cfg(test)]
            path: path_buf,
        };
        mutation::recover(&directory)?;
        Ok(directory)
    }

    pub(super) fn open_file(&self, name: &str, write: bool) -> io::Result<File> {
        let name = CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))?;
        let mut flags = libc::O_CLOEXEC | libc::O_NOFOLLOW;
        flags |= if write { libc::O_RDWR } else { libc::O_RDONLY };
        let descriptor = unsafe { libc::openat(self.descriptor.as_raw_fd(), name.as_ptr(), flags) };
        if descriptor < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(descriptor) })
        }
    }

    pub(super) fn create_file(&self, name: &str) -> io::Result<File> {
        let name = CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))?;
        let descriptor = unsafe {
            libc::openat(
                self.descriptor.as_raw_fd(),
                name.as_ptr(),
                libc::O_CREAT | libc::O_EXCL | libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if descriptor < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(descriptor) })
        }
    }

    pub(super) fn validate(
        &self,
        name: &str,
        expected: FileIdentity,
        private: bool,
    ) -> RevisionResult<()> {
        let actual = self.open_file(name, false)?;
        if validated_identity(&actual, private)? != expected {
            return Err(RevisionError::Invalid(
                "private publication slot changed before exchange".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn remove(&self, name: &str) -> io::Result<()> {
        let name = CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))?;
        let removed = unsafe { libc::unlinkat(self.descriptor.as_raw_fd(), name.as_ptr(), 0) };
        if removed == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub(super) fn sync(&self) -> RevisionResult<()> {
        self.descriptor.sync_all()?;
        Ok(())
    }

    pub(super) fn write_marker(&self, name: &str, bytes: &[u8]) -> RevisionResult<()> {
        self.write_marker_with_boundaries(name, bytes, None)
    }

    fn write_marker_with_boundaries(
        &self,
        name: &str,
        bytes: &[u8],
        faults: Option<(super::PublishFault, super::PublishFault)>,
    ) -> RevisionResult<()> {
        let temporary = format!(".{name}-{}.tmp", SessionId::new());
        let result = (|| -> RevisionResult<()> {
            let mut marker = self.create_file(&temporary)?;
            marker.write_all(bytes)?;
            marker.sync_all()?;
            let identity = validated_identity(&marker, true)?;
            self.validate(&temporary, identity, true)?;
            self.rename(&temporary, name)?;
            if let Some((renamed, _)) = faults {
                super::maybe_recovery_fault(renamed)?;
            }
            self.sync()?;
            if let Some((_, synced)) = faults {
                super::maybe_recovery_fault(synced)?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = self.remove(&temporary);
            let _ = self.sync();
        }
        result
    }

    pub(super) fn read_marker(&self, name: &str) -> RevisionResult<Vec<u8>> {
        let mut marker = self.open_file(name, false)?;
        validated_identity(&marker, true)?;
        if marker.metadata()?.len() > 1024 {
            return Err(RevisionError::Corrupt(
                "publication marker exceeds its bounded format".into(),
            ));
        }
        let mut bytes = Vec::new();
        marker.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub(super) fn rename(&self, source: &str, destination: &str) -> RevisionResult<()> {
        let source = CString::new(source)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))?;
        let destination = CString::new(destination)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))?;
        let renamed = unsafe {
            libc::renameat(
                self.descriptor.as_raw_fd(),
                source.as_ptr(),
                self.descriptor.as_raw_fd(),
                destination.as_ptr(),
            )
        };
        if renamed == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error().into())
        }
    }

    pub(super) fn exchange(
        &self,
        source_name: &str,
        source_identity: FileIdentity,
        destination: &Path,
        destination_identity: FileIdentity,
    ) -> RevisionResult<ExchangeOutcome> {
        self.validate(source_name, source_identity, true)?;
        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = CString::new(parent.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory contains NUL"))?;
        let parent_descriptor = unsafe {
            libc::open(
                parent.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if parent_descriptor < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let parent_descriptor = unsafe { File::from_raw_fd(parent_descriptor) };
        let destination_name = destination
            .file_name()
            .ok_or_else(|| RevisionError::Invalid("canonical project has no file name".into()))?;
        let destination_name = CString::new(destination_name.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))?;
        let canonical =
            super::openat_nofollow(&parent_descriptor, destination_name.as_c_str(), false)?;
        if validated_identity(&canonical, false)? != destination_identity {
            return Err(RevisionError::Invalid(
                "canonical project changed immediately before exchange".into(),
            ));
        }
        use std::os::unix::fs::MetadataExt as _;
        if canonical.metadata()?.nlink() != 1 {
            return Ok(ExchangeOutcome::CanonicalLinked);
        }
        self.validate(source_name, source_identity, true)?;
        maybe_inject_canonical_hardlink(destination)?;
        let source_name = CString::new(source_name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))?;
        let exchanged = unsafe {
            libc::renameat2(
                self.descriptor.as_raw_fd(),
                source_name.as_ptr(),
                parent_descriptor.as_raw_fd(),
                destination_name.as_ptr(),
                RENAME_EXCHANGE,
            )
        };
        if exchanged != 0 {
            return Err(io::Error::last_os_error().into());
        }
        parent_descriptor.sync_all()?;
        self.sync()?;
        let published =
            super::openat_nofollow(&parent_descriptor, destination_name.as_c_str(), false)?;
        if validated_identity(&published, false)? != source_identity {
            return Err(RevisionError::Invalid(
                "canonical project does not contain the exchanged candidate".into(),
            ));
        }
        let old_slot = self.open_file(
            source_name
                .to_str()
                .map_err(|_| RevisionError::Invalid("file name is not UTF-8".into()))?,
            false,
        )?;
        if validated_identity(&old_slot, false)? != destination_identity {
            return Err(RevisionError::Invalid(
                "private slot does not contain the exchanged canonical".into(),
            ));
        }
        Ok(ExchangeOutcome::Exchanged {
            old_slot_private: old_slot.metadata()?.nlink() == 1,
        })
    }

    pub(super) fn exchange_supported(&self) -> RevisionResult<bool> {
        // Probe entries are randomized scratch, never publication or recovery state. Namespace
        // success is sufficient to detect rename support, so durability barriers do not belong here.
        let first_name = format!(".exchange-probe-a-{}", SessionId::new());
        let second_name = format!(".exchange-probe-b-{}", SessionId::new());
        let _first = self.create_file(&first_name)?;
        let _second = match self.create_file(&second_name) {
            Ok(second) => second,
            Err(error) => {
                let _ = self.remove(&first_name);
                return Err(error.into());
            }
        };
        let first_name_c = CString::new(first_name.as_str()).unwrap();
        let second_name_c = CString::new(second_name.as_str()).unwrap();
        let result = unsafe {
            libc::renameat2(
                self.descriptor.as_raw_fd(),
                first_name_c.as_ptr(),
                self.descriptor.as_raw_fd(),
                second_name_c.as_ptr(),
                RENAME_EXCHANGE,
            )
        };
        let error = io::Error::last_os_error();
        let _ = self.remove(&first_name);
        let _ = self.remove(&second_name);
        if result == 0 {
            Ok(true)
        } else if error.raw_os_error().is_some_and(|code| {
            code == libc::ENOSYS || code == libc::EINVAL || code == libc::EOPNOTSUPP
        }) {
            Ok(false)
        } else {
            Err(error.into())
        }
    }

    pub(super) fn device(&self) -> RevisionResult<u64> {
        use std::os::unix::fs::MetadataExt as _;
        Ok(self.descriptor.metadata()?.dev())
    }

    pub(super) fn begin_mutation(
        &self,
        name: &str,
        descriptor: &File,
        identity: FileIdentity,
        point: SlotMutationPoint,
    ) -> RevisionResult<mutation::PrivateMutation<'_>> {
        mutation::PrivateMutation::begin(self, name, descriptor, identity, point)
    }
}

#[cfg(test)]
fn maybe_inject_slot_hardlink(
    directory: &PrivateDirectory,
    name: &str,
    point: SlotMutationPoint,
) -> RevisionResult<()> {
    let alias = SLOT_HARDLINK_RACE.with(|hook| {
        let mut hook = hook.borrow_mut();
        match hook.as_ref() {
            Some((configured, _)) if *configured == point => hook.take().map(|(_, alias)| alias),
            _ => None,
        }
    });
    if let Some(alias) = alias {
        fs::hard_link(directory.path.join(name), alias)?;
    }
    Ok(())
}

#[cfg(not(test))]
fn maybe_inject_slot_hardlink(
    _directory: &PrivateDirectory,
    _name: &str,
    _point: SlotMutationPoint,
) -> RevisionResult<()> {
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct ExchangeIntent {
    pub(super) canonical_identity: FileIdentity,
    pub(super) candidate_identity: FileIdentity,
    pub(super) generation: u64,
    pub(super) state_id: Option<StorageStateId>,
    pub(super) target_generation: u64,
    pub(super) target_state_id: Option<StorageStateId>,
}

impl ExchangeIntent {
    const MAGIC: [u8; 8] = *b"SPXCHG01";
    const ENCODED_LEN: usize = 8 + 6 * 8 + 2 * (1 + 16);

    pub(super) fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(Self::ENCODED_LEN);
        bytes.extend(Self::MAGIC);
        for value in [
            self.canonical_identity.device,
            self.canonical_identity.inode,
            self.candidate_identity.device,
            self.candidate_identity.inode,
            self.generation,
            self.target_generation,
        ] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.push(u8::from(self.state_id.is_some()));
        bytes.extend(self.state_id.unwrap_or([0; 16]));
        bytes.push(u8::from(self.target_state_id.is_some()));
        bytes.extend(self.target_state_id.unwrap_or([0; 16]));
        bytes
    }

    pub(super) fn decode(bytes: &[u8]) -> RevisionResult<Self> {
        if bytes.len() != Self::ENCODED_LEN || bytes[..8] != Self::MAGIC {
            return Err(RevisionError::Corrupt(
                "invalid publication exchange intent".into(),
            ));
        }
        let mut offset = 8;
        let mut next_u64 = || {
            let value = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
            offset += 8;
            value
        };
        let canonical_identity = FileIdentity {
            device: next_u64(),
            inode: next_u64(),
        };
        let candidate_identity = FileIdentity {
            device: next_u64(),
            inode: next_u64(),
        };
        let generation = next_u64();
        let target_generation = next_u64();
        let present = bytes[offset];
        offset += 1;
        let state_bytes: [u8; 16] = bytes[offset..offset + 16].try_into().unwrap();
        let state_id = match present {
            0 if state_bytes == [0; 16] => None,
            0 => {
                return Err(RevisionError::Corrupt(
                    "absent publication state has a non-canonical payload".into(),
                ));
            }
            1 => Some(state_bytes),
            _ => {
                return Err(RevisionError::Corrupt(
                    "invalid publication exchange state marker".into(),
                ));
            }
        };
        offset += 16;
        let target_present = bytes[offset];
        offset += 1;
        let target_state_bytes: [u8; 16] = bytes[offset..offset + 16].try_into().unwrap();
        let target_state_id = match target_present {
            0 if target_state_bytes == [0; 16] => None,
            0 => {
                return Err(RevisionError::Corrupt(
                    "absent publication target state has a non-canonical payload".into(),
                ));
            }
            1 => Some(target_state_bytes),
            _ => {
                return Err(RevisionError::Corrupt(
                    "invalid publication target state marker".into(),
                ));
            }
        };
        let decoded = Self {
            canonical_identity,
            candidate_identity,
            generation,
            state_id,
            target_generation,
            target_state_id,
        };
        if decoded.canonical_identity == decoded.candidate_identity {
            return Err(RevisionError::Corrupt(
                "publication exchange identities must be distinct".into(),
            ));
        }
        if decoded.target_generation <= decoded.generation {
            return Err(RevisionError::Corrupt(
                "publication exchange generation must advance".into(),
            ));
        }
        Ok(decoded)
    }
}

pub(super) fn inspect_named_checkpoint(
    path: &Path,
    descriptor: &File,
    identity: FileIdentity,
    private: bool,
) -> RevisionResult<crate::store::StoreInspection> {
    if validated_identity(descriptor, private)? != identity {
        return Err(RevisionError::Invalid(
            "publication descriptor changed before inspection".into(),
        ));
    }
    let inspection = RevisionStore::inspect(path)?;
    if validated_identity(descriptor, private)? != identity {
        return Err(RevisionError::Invalid(
            "publication descriptor changed during inspection".into(),
        ));
    }
    validate_named_identity(path, identity, private)?;
    Ok(inspection)
}

pub(super) fn recover_exchange(
    directory: &PrivateDirectory,
    destination: &Path,
    cache_directory: &Path,
) -> RevisionResult<()> {
    let intent = match directory.read_marker(super::PUBLISH_EXCHANGE_INTENT_FILE) {
        Ok(bytes) => ExchangeIntent::decode(&bytes)?,
        Err(RevisionError::Io(error)) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let canonical = super::open_nofollow(destination, false)?;
    let canonical_identity = validated_identity(&canonical, false)?;
    let canonical_inspection =
        inspect_named_checkpoint(destination, &canonical, canonical_identity, false)?;
    let slot = match directory.open_file(super::PUBLISH_MIRROR_FILE, false) {
        Ok(slot) => slot,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if canonical_identity != intent.candidate_identity
                || canonical_inspection.generation != intent.target_generation
                || canonical_inspection.state_id != intent.target_state_id
            {
                return Err(RevisionError::Invalid(
                    "missing publication slot does not match a committed exchange".into(),
                ));
            }
            write_publication_marker(directory, intent.target_generation, intent.target_state_id)?;
            remove_private_file(directory, super::PUBLISH_MIRROR_READY_FILE)?;
            remove_private_file(directory, super::PUBLISH_EXCHANGE_INTENT_FILE)?;
            directory.sync()?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let slot_identity = validated_identity(&slot, false)?;
    use std::os::unix::fs::MetadataExt as _;
    let slot_private = slot.metadata()?.nlink() == 1;
    let slot_path = cache_directory.join(super::PUBLISH_MIRROR_FILE);
    let slot_inspection = inspect_named_checkpoint(&slot_path, &slot, slot_identity, false)?;
    if canonical_identity == intent.candidate_identity
        && slot_identity == intent.canonical_identity
        && canonical_inspection.generation == intent.target_generation
        && canonical_inspection.state_id == intent.target_state_id
        && slot_inspection.generation == intent.generation
        && slot_inspection.state_id == intent.state_id
    {
        if slot_private {
            prepare_reusable_slot(
                directory,
                super::PUBLISH_MIRROR_FILE,
                &slot,
                intent.canonical_identity,
            )?;
            write_ready_marker(directory, intent.generation, intent.state_id)?;
        } else {
            remove_private_file(directory, super::PUBLISH_MIRROR_FILE)?;
            remove_private_file(directory, super::PUBLISH_MIRROR_READY_FILE)?;
        }
        write_publication_marker(directory, intent.target_generation, intent.target_state_id)?;
    } else if canonical_identity == intent.canonical_identity
        && slot_identity == intent.candidate_identity
        && canonical_inspection.generation == intent.generation
        && canonical_inspection.state_id == intent.state_id
        && slot_inspection.generation == intent.target_generation
        && slot_inspection.state_id == intent.target_state_id
    {
        prepare_reusable_slot(
            directory,
            super::PUBLISH_MIRROR_FILE,
            &slot,
            intent.candidate_identity,
        )?;
        remove_private_file(directory, super::PUBLISH_MIRROR_READY_FILE)?;
        write_publication_marker(directory, intent.generation, intent.state_id)?;
    } else {
        return Err(RevisionError::Invalid(
            "publication exchange residuals do not match either atomic state".into(),
        ));
    }
    remove_private_file(directory, super::PUBLISH_EXCHANGE_INTENT_FILE)?;
    directory.sync()
}

pub(super) fn remove_private_file(directory: &PrivateDirectory, name: &str) -> RevisionResult<()> {
    match directory.remove(name) {
        Ok(()) => directory.sync(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn remove_private_file_lazy(
    directory: &PrivateDirectory,
    name: &str,
) -> RevisionResult<()> {
    match directory.remove(name) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn prepare_reusable_slot(
    directory: &PrivateDirectory,
    name: &str,
    descriptor: &File,
    identity: FileIdentity,
) -> RevisionResult<()> {
    use std::os::unix::fs::MetadataExt as _;

    let mutation = directory.begin_mutation(
        name,
        descriptor,
        identity,
        SlotMutationPoint::PermissionRepair,
    )?;
    if descriptor.metadata()?.mode() & 0o200 == 0 {
        descriptor.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    descriptor.sync_all()?;
    validated_identity(descriptor, true)?;
    mutation.finish(name, identity)
}

fn apply_checkpoint_delta_from(source: &File, mirror: &File) -> RevisionResult<PublishStats> {
    use std::os::unix::fs::FileExt as _;

    validated_identity(source, false)?;
    let source_len = source.metadata()?.len();
    let mirror_len = mirror.metadata()?.len();
    let mut source_chunk = vec![0; super::WRITE_BLOCK_BYTES];
    let mut mirror_chunk = vec![0; super::WRITE_BLOCK_BYTES];
    let mut offset = 0_u64;
    let mut changed_bytes = mirror_len.saturating_sub(source_len);
    let mut written_bytes = 0_u64;
    while offset < source_len {
        let chunk_len = usize::try_from((source_len - offset).min(super::WRITE_BLOCK_BYTES as u64))
            .expect("bounded comparison chunk");
        source.read_exact_at(&mut source_chunk[..chunk_len], offset)?;
        mirror_chunk[..chunk_len].fill(0);
        let overlap_len =
            usize::try_from((mirror_len.saturating_sub(offset)).min(chunk_len as u64))
                .expect("bounded comparison chunk");
        if overlap_len > 0 {
            mirror.read_exact_at(&mut mirror_chunk[..overlap_len], offset)?;
        }
        let overlap_changed = source_chunk[..overlap_len] != mirror_chunk[..overlap_len];
        let new_tail_len = chunk_len - overlap_len;
        if overlap_changed {
            changed_bytes += overlap_len as u64;
        }
        changed_bytes += new_tail_len as u64;
        if overlap_changed || new_tail_len > 0 {
            mirror.write_all_at(&source_chunk[..chunk_len], offset)?;
            written_bytes += chunk_len as u64;
        }
        offset += chunk_len as u64;
    }
    if source_len != mirror_len {
        mirror.set_len(source_len)?;
    }
    Ok(PublishStats {
        incremental: true,
        reflink_unavailable: false,
        strategy: PublishStrategy::PageDiffExchange,
        scanned_bytes: source_len.max(mirror_len),
        changed_bytes,
        written_bytes,
        timings: PublishTimings::default(),
    })
}

#[cfg(test)]
pub(super) fn apply_checkpoint_delta(source: &Path, mirror: &File) -> RevisionResult<PublishStats> {
    let source = super::open_nofollow(source, false)?;
    apply_checkpoint_delta_from(&source, mirror)
}

pub(super) fn update_private_slot(
    directory: &PrivateDirectory,
    name: &str,
    source: &Path,
    mirror: &File,
    identity: FileIdentity,
    permissions: Option<fs::Permissions>,
    point: SlotMutationPoint,
) -> RevisionResult<PublishStats> {
    let source = super::open_nofollow(source, false)?;
    validated_identity(&source, false)?;
    let mutation = directory.begin_mutation(name, mirror, identity, point)?;
    let stats = apply_checkpoint_delta_from(&source, mirror)?;
    if let Some(permissions) = permissions {
        mirror.set_permissions(permissions)?;
    }
    mirror.sync_all()?;
    validated_identity(mirror, true)?;
    mutation.finish(name, identity)?;
    Ok(stats)
}

pub(super) fn update_candidate_slot(
    directory: &PrivateDirectory,
    source: &Path,
    mirror: &File,
    identity: FileIdentity,
    permissions: fs::Permissions,
) -> RevisionResult<PublishStats> {
    update_private_slot(
        directory,
        super::PUBLISH_MIRROR_FILE,
        source,
        mirror,
        identity,
        Some(permissions),
        SlotMutationPoint::CandidateDelta,
    )
}

pub(super) fn catch_up_after_bulk(
    directory: &PrivateDirectory,
    source: &Path,
    mut stats: PublishStats,
    generation: u64,
    state_id: Option<StorageStateId>,
) -> RevisionResult<PublishStats> {
    if stats.changed_bytes < super::BULK_CHANGE_CATCH_UP_BYTES
        || stats.changed_bytes.saturating_mul(4) < stats.scanned_bytes
    {
        return Ok(stats);
    }
    let slot = directory.open_file(super::PUBLISH_MIRROR_FILE, true)?;
    let identity = validated_identity(&slot, true)?;
    let catch_up = update_private_slot(
        directory,
        super::PUBLISH_MIRROR_FILE,
        source,
        &slot,
        identity,
        None,
        SlotMutationPoint::BulkCatchUp,
    )?;
    write_ready_marker(directory, generation, state_id)?;
    stats.scanned_bytes = stats.scanned_bytes.saturating_add(catch_up.scanned_bytes);
    stats.changed_bytes = stats.changed_bytes.saturating_add(catch_up.changed_bytes);
    stats.written_bytes = stats.written_bytes.saturating_add(catch_up.written_bytes);
    Ok(stats)
}

pub(super) fn seed_incremental_mirror(
    destination: &Path,
    cache_directory: &Path,
    generation: u64,
    state_id: Option<StorageStateId>,
) {
    let Ok(directory) = PrivateDirectory::open(cache_directory) else {
        return;
    };
    use std::os::unix::fs::MetadataExt as _;
    let same_device = match (destination.metadata(), directory.device()) {
        (Ok(metadata), Ok(device)) => metadata.dev() == device,
        _ => false,
    };
    if !same_device {
        return;
    }
    let _ = remove_private_file(&directory, super::PUBLISH_MIRROR_READY_FILE);
    let _ = remove_private_file(&directory, super::PUBLISH_EXCHANGE_INTENT_FILE);
    let temporary = format!(".published-mirror-{}.tmp", SessionId::new());
    let seeded = (|| -> RevisionResult<()> {
        let mut candidate = directory.create_file(&temporary)?;
        let mut source = super::open_nofollow(destination, false)?;
        io::copy(&mut source, &mut candidate)?;
        candidate.set_permissions(fs::Permissions::from_mode(0o600))?;
        candidate.sync_all()?;
        let identity = validated_identity(&candidate, true)?;
        directory.validate(&temporary, identity, true)?;
        remove_private_file(&directory, super::PUBLISH_MIRROR_FILE)?;
        directory.rename(&temporary, super::PUBLISH_MIRROR_FILE)?;
        directory.sync()?;
        super::maybe_seed_fault()?;
        write_ready_marker(&directory, generation, state_id)?;
        directory.sync()?;
        Ok(())
    })();
    if seeded.is_err() {
        let _ = directory.remove(&temporary);
        let _ = directory.remove(super::PUBLISH_MIRROR_FILE);
        let _ = directory.remove(super::PUBLISH_MIRROR_READY_FILE);
        let _ = directory.sync();
    }
}

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
