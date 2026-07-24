use std::{
    fs::File,
    io,
    os::{
        fd::{AsRawFd as _, RawFd},
        unix::fs::MetadataExt as _,
    },
    path::Path,
};

use super::{
    ExchangeIntent, PrivateDirectory, RevisionError, RevisionResult, StorageStateId,
    inspect_named_checkpoint, prepare_reusable_slot, remove_private_file, remove_private_file_lazy,
    validate_named_identity, validated_identity,
};

const EXCHANGE_INTENT_XATTR: &std::ffi::CStr = c"user.spectrum.exchange-intent-v2";

fn exchange_xattr_unsupported(error: &io::Error) -> bool {
    error.raw_os_error().is_some_and(|code| {
        code == libc::ENOTSUP
            || code == libc::EOPNOTSUPP
            || code == libc::ENOSYS
            || code == libc::EPERM
    })
}

pub(in crate::live) fn write_exchange_intent(
    descriptor: &File,
    intent: ExchangeIntent,
) -> RevisionResult<bool> {
    let bytes = intent.encode();
    let written = unsafe {
        libc::fsetxattr(
            descriptor.as_raw_fd(),
            EXCHANGE_INTENT_XATTR.as_ptr(),
            bytes.as_ptr().cast(),
            bytes.len(),
            0,
        )
    };
    if written == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if exchange_xattr_unsupported(&error) {
        Ok(false)
    } else {
        Err(error.into())
    }
}

fn read_exchange_intent(descriptor: &File) -> RevisionResult<Option<ExchangeIntent>> {
    read_exchange_intent_fd(descriptor.as_raw_fd())
}

fn read_exchange_intent_fd(descriptor: RawFd) -> RevisionResult<Option<ExchangeIntent>> {
    let mut bytes = [0_u8; ExchangeIntent::ENCODED_LEN];
    let read = unsafe {
        libc::fgetxattr(
            descriptor,
            EXCHANGE_INTENT_XATTR.as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if read < 0 {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ENODATA) || exchange_xattr_unsupported(&error)
        {
            Ok(None)
        } else {
            Err(error.into())
        };
    }
    if usize::try_from(read).ok() != Some(bytes.len()) {
        return Err(RevisionError::Corrupt(
            "invalid inode-bound publication exchange intent".into(),
        ));
    }
    ExchangeIntent::decode(&bytes).map(Some)
}

pub(in crate::live) fn exchange_proof_matches(
    destination: &Path,
    generation: u64,
    state_id: Option<StorageStateId>,
) -> RevisionResult<bool> {
    let canonical = match super::super::open_nofollow(destination, false) {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let identity = validated_identity(&canonical, false)?;
    let Some(intent) = read_exchange_intent(&canonical)? else {
        return Ok(false);
    };
    validate_named_identity(destination, identity, false)?;
    Ok(intent.candidate_identity == identity
        && intent.target_generation == generation
        && intent.target_state_id == state_id)
}

pub(in crate::live) fn recover_inode_exchange(
    directory: &PrivateDirectory,
    destination: &Path,
    cache_directory: &Path,
) -> RevisionResult<()> {
    let canonical = match super::super::open_nofollow(destination, false) {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let slot = match directory.open_file(super::super::PUBLISH_MIRROR_FILE, false) {
                Ok(slot) => Some(slot),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            if slot
                .as_ref()
                .map(read_exchange_intent)
                .transpose()?
                .flatten()
                .is_some()
            {
                return Err(RevisionError::Invalid(
                    "inode-bound publication intent has no canonical predecessor".into(),
                ));
            }
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let canonical_identity = validated_identity(&canonical, false)?;
    let canonical_inspection =
        inspect_named_checkpoint(destination, &canonical, canonical_identity, false)?;
    let canonical_intent = read_exchange_intent(&canonical)?;
    validate_named_identity(destination, canonical_identity, false)?;
    let slot = match directory.open_file(super::super::PUBLISH_MIRROR_FILE, false) {
        Ok(slot) => Some(slot),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let slot_identity = slot
        .as_ref()
        .map(|slot| validated_identity(slot, false))
        .transpose()?;
    let slot_intent = slot
        .as_ref()
        .map(read_exchange_intent)
        .transpose()?
        .flatten();

    if let (Some(slot), Some(slot_identity), Some(intent)) =
        (slot.as_ref(), slot_identity, slot_intent)
    {
        let active_pre_exchange = intent.candidate_identity == slot_identity
            && intent.target_generation > canonical_inspection.generation;
        if active_pre_exchange {
            if intent.canonical_identity != canonical_identity
                || intent.generation != canonical_inspection.generation
                || intent.state_id != canonical_inspection.state_id
            {
                return Err(RevisionError::Invalid(
                    "inode-bound publication intent does not match its canonical predecessor"
                        .into(),
                ));
            }
            prepare_reusable_slot(
                directory,
                super::super::PUBLISH_MIRROR_FILE,
                slot,
                slot_identity,
            )?;
            remove_private_file_lazy(directory, super::super::PUBLISH_MIRROR_READY_FILE)?;
            return Ok(());
        }
        if intent.target_generation > canonical_inspection.generation {
            return Err(RevisionError::Invalid(
                "newer inode-bound publication intent names a different candidate".into(),
            ));
        }
    }

    if let Some(intent) = canonical_intent {
        if intent.candidate_identity != canonical_identity
            || intent.target_generation != canonical_inspection.generation
            || intent.target_state_id != canonical_inspection.state_id
        {
            return Ok(());
        }
        if let (Some(slot), Some(slot_identity)) = (slot.as_ref(), slot_identity) {
            if intent.canonical_identity != slot_identity {
                return Err(RevisionError::Invalid(
                    "committed inode-bound publication proof names a different predecessor".into(),
                ));
            }
            let slot_path = cache_directory.join(super::super::PUBLISH_MIRROR_FILE);
            let slot_inspection =
                match inspect_named_checkpoint(&slot_path, slot, slot_identity, false) {
                    Ok(inspection) => inspection,
                    Err(_) => {
                        directory.validate(
                            super::super::PUBLISH_MIRROR_FILE,
                            slot_identity,
                            false,
                        )?;
                        if slot.metadata()?.nlink() == 1 {
                            prepare_reusable_slot(
                                directory,
                                super::super::PUBLISH_MIRROR_FILE,
                                slot,
                                slot_identity,
                            )?;
                        } else {
                            remove_private_file(directory, super::super::PUBLISH_MIRROR_FILE)?;
                        }
                        remove_private_file_lazy(
                            directory,
                            super::super::PUBLISH_MIRROR_READY_FILE,
                        )?;
                        return Ok(());
                    }
                };
            let predecessor = slot_inspection.generation == intent.generation
                && slot_inspection.state_id == intent.state_id;
            let caught_up = slot_inspection.generation == intent.target_generation
                && slot_inspection.state_id == intent.target_state_id;
            // The exact committed canonical proof and exact predecessor inode
            // identity above are the authority boundary: only here may valid
            // older slot bytes be treated as non-authorizing reusable scratch.
            let stale_non_authorizing =
                slot_inspection.generation <= canonical_inspection.generation;
            if !predecessor && !caught_up && !stale_non_authorizing {
                return Err(RevisionError::Invalid(
                    "committed inode-bound publication slot is newer than its canonical target"
                        .into(),
                ));
            }
            if slot.metadata()?.nlink() == 1 {
                prepare_reusable_slot(
                    directory,
                    super::super::PUBLISH_MIRROR_FILE,
                    slot,
                    slot_identity,
                )?;
            } else {
                remove_private_file(directory, super::super::PUBLISH_MIRROR_FILE)?;
            }
        }
    }
    Ok(())
}
