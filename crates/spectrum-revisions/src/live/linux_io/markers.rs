use std::io;

use super::{PrivateDirectory, RevisionError, RevisionResult, StorageStateId};

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

#[cfg(test)]
pub(in crate::live) fn ready_marker_matches(
    directory: &PrivateDirectory,
    generation: u64,
    state_id: Option<StorageStateId>,
) -> bool {
    directory
        .read_marker(super::super::PUBLISH_MIRROR_READY_FILE)
        .is_ok_and(|bytes| bytes == marker_bytes(generation, state_id))
}

pub(in crate::live) fn write_ready_marker(
    directory: &PrivateDirectory,
    generation: u64,
    state_id: Option<StorageStateId>,
) -> RevisionResult<()> {
    directory.write_marker(
        super::super::PUBLISH_MIRROR_READY_FILE,
        &marker_bytes(generation, state_id),
    )
}

pub(in crate::live) fn publication_marker_matches(
    directory: &PrivateDirectory,
    generation: u64,
    state_id: Option<StorageStateId>,
) -> bool {
    directory
        .read_marker(super::super::PUBLISH_CURRENT_FILE)
        .is_ok_and(|bytes| bytes == marker_bytes(generation, state_id))
}

pub(in crate::live) fn read_publication_marker(
    directory: &PrivateDirectory,
) -> RevisionResult<Option<Vec<u8>>> {
    match directory.read_marker(super::super::PUBLISH_CURRENT_FILE) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(RevisionError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(in crate::live) fn publication_marker_bytes_match(
    bytes: &[u8],
    generation: u64,
    state_id: Option<StorageStateId>,
) -> bool {
    bytes == marker_bytes(generation, state_id)
}

pub(in crate::live) fn write_publication_marker(
    directory: &PrivateDirectory,
    generation: u64,
    state_id: Option<StorageStateId>,
) -> RevisionResult<()> {
    directory.write_marker(
        super::super::PUBLISH_CURRENT_FILE,
        &marker_bytes(generation, state_id),
    )
}

#[cfg(test)]
pub(in crate::live) fn working_recovery_marker_matches(
    directory: &PrivateDirectory,
    generation: u64,
    state_id: Option<StorageStateId>,
) -> bool {
    directory
        .read_marker(super::super::PUBLISH_WORKING_RECOVERY_FILE)
        .is_ok_and(|bytes| bytes == marker_bytes(generation, state_id))
}

pub(in crate::live) fn read_working_recovery_marker(
    directory: &PrivateDirectory,
) -> RevisionResult<Option<Vec<u8>>> {
    match directory.read_marker(super::super::PUBLISH_WORKING_RECOVERY_FILE) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(RevisionError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(in crate::live) fn working_recovery_marker_bytes_match(
    bytes: &[u8],
    generation: u64,
    state_id: Option<StorageStateId>,
) -> bool {
    bytes == marker_bytes(generation, state_id)
}

pub(in crate::live) fn working_recovery_poison_present(
    directory: &PrivateDirectory,
) -> RevisionResult<bool> {
    match directory.read_marker(super::super::PUBLISH_WORKING_POISON_FILE) {
        Ok(_) => Ok(true),
        Err(RevisionError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(in crate::live) enum WorkingRecoveryInstall {
    Clean,
    PoisonCleanupPending(String),
}

pub(in crate::live) fn write_working_recovery_marker(
    directory: &PrivateDirectory,
    generation: u64,
    state_id: Option<StorageStateId>,
) -> RevisionResult<WorkingRecoveryInstall> {
    directory.write_marker_with_boundaries(
        super::super::PUBLISH_WORKING_POISON_FILE,
        &marker_bytes(generation, state_id),
        Some((
            super::super::PublishFault::WorkingPoisonRenamed,
            super::super::PublishFault::WorkingPoisonSynced,
        )),
    )?;
    directory.write_marker_with_boundaries(
        super::super::PUBLISH_WORKING_RECOVERY_FILE,
        &marker_bytes(generation, state_id),
        Some((
            super::super::PublishFault::WorkingRecoveryMarkerRenamed,
            super::super::PublishFault::WorkingRecoveryMarkerSynced,
        )),
    )?;
    let cleanup = (|| {
        match directory.remove(super::super::PUBLISH_WORKING_POISON_FILE) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(RevisionError::Io(error)),
        }
        super::super::maybe_recovery_fault(super::super::PublishFault::WorkingPoisonRemoved)?;
        directory.sync()?;
        super::super::maybe_recovery_fault(super::super::PublishFault::WorkingPoisonRemovalSynced)
    })();
    match cleanup {
        Ok(()) => Ok(WorkingRecoveryInstall::Clean),
        Err(error) => Ok(WorkingRecoveryInstall::PoisonCleanupPending(
            error.to_string(),
        )),
    }
}
