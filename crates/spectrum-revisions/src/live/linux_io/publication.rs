use std::{
    fs::{self, File},
    io,
    path::Path,
};

use super::{PrivateDirectory, publication_marker_matches};
use crate::live::{
    FileIdentity, RevisionError, RevisionResult, RevisionStore, StorageStateId, file_identity,
    open_nofollow, validated_identity,
};

pub(in crate::live) struct PublishBase {
    pub(in crate::live) canonical: Option<File>,
    pub(in crate::live) identity: Option<FileIdentity>,
    pub(in crate::live) permissions: Option<fs::Permissions>,
    pub(in crate::live) generation: u64,
    pub(in crate::live) state_id: Option<StorageStateId>,
}

pub(in crate::live) fn validate_publish_base(
    destination: &Path,
    cache_directory: &Path,
    expected_generation: u64,
    expected_state_id: Option<StorageStateId>,
    source_generation: u64,
    source_state_id: Option<StorageStateId>,
) -> RevisionResult<PublishBase> {
    let canonical = match open_nofollow(destination, false) {
        Ok(file) => file,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                && expected_generation == 0
                && expected_state_id.is_none() =>
        {
            return Ok(PublishBase {
                canonical: None,
                identity: None,
                permissions: None,
                generation: 0,
                state_id: None,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let metadata = canonical.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(RevisionError::Invalid(
            "canonical project is not a regular file".into(),
        ));
    }
    let identity = file_identity(&metadata);
    let inspection = RevisionStore::inspect(destination)?;
    let revalidated = open_nofollow(destination, false)?;
    if validated_identity(&revalidated, false)? != identity {
        return Err(RevisionError::Invalid(
            "canonical project changed while it was inspected".into(),
        ));
    }
    let expected_matches =
        inspection.generation == expected_generation && inspection.state_id == expected_state_id;
    let already_published =
        inspection.generation == source_generation && inspection.state_id == source_state_id;
    let shared_cache_advanced = inspection.generation > expected_generation
        && inspection.generation < source_generation
        && publication_marker_matches(
            &PrivateDirectory::open(cache_directory)?,
            inspection.generation,
            inspection.state_id,
        );
    if !expected_matches && !already_published && !shared_cache_advanced {
        return Err(RevisionError::Invalid(format!(
            "project publication conflict: expected generation {expected_generation}, found {}",
            inspection.generation
        )));
    }
    Ok(PublishBase {
        canonical: Some(canonical),
        identity: Some(identity),
        permissions: Some(metadata.permissions()),
        generation: inspection.generation,
        state_id: inspection.state_id,
    })
}
