use std::{
    fs::{self, File},
    io,
    path::Path,
};

use super::{
    PrivateDirectory, exchange_proof_matches, publication_marker_matches, remove_private_file,
    write_publication_marker,
};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FullCopyIntent {
    generation: u64,
    state_id: Option<StorageStateId>,
    target_generation: u64,
    target_state_id: Option<StorageStateId>,
}

impl FullCopyIntent {
    const ENCODED_LEN: usize = 58;
    const MAGIC: &'static [u8; 8] = b"SPCOPY01";

    fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(Self::ENCODED_LEN);
        bytes.extend(Self::MAGIC);
        bytes.extend(self.generation.to_le_bytes());
        bytes.extend(self.target_generation.to_le_bytes());
        encode_state(&mut bytes, self.state_id);
        encode_state(&mut bytes, self.target_state_id);
        bytes
    }

    fn decode(bytes: &[u8]) -> RevisionResult<Self> {
        if bytes.len() != Self::ENCODED_LEN || &bytes[..8] != Self::MAGIC {
            return Err(RevisionError::Corrupt(
                "invalid full-copy publication intent".into(),
            ));
        }
        let generation = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let target_generation = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let state_id = decode_state(&bytes[24..41])?;
        let target_state_id = decode_state(&bytes[41..58])?;
        if target_generation <= generation {
            return Err(RevisionError::Corrupt(
                "full-copy publication generation must advance".into(),
            ));
        }
        Ok(Self {
            generation,
            state_id,
            target_generation,
            target_state_id,
        })
    }
}

fn encode_state(bytes: &mut Vec<u8>, state_id: Option<StorageStateId>) {
    bytes.push(u8::from(state_id.is_some()));
    bytes.extend(state_id.unwrap_or([0; 16]));
}

fn decode_state(bytes: &[u8]) -> RevisionResult<Option<StorageStateId>> {
    let state_id: StorageStateId = bytes[1..17].try_into().unwrap();
    match bytes[0] {
        0 if state_id == [0; 16] => Ok(None),
        0 => Err(RevisionError::Corrupt(
            "absent full-copy publication state has a non-canonical payload".into(),
        )),
        1 => Ok(Some(state_id)),
        _ => Err(RevisionError::Corrupt(
            "invalid full-copy publication state marker".into(),
        )),
    }
}

pub(in crate::live) fn write_full_copy_intent(
    directory: &PrivateDirectory,
    generation: u64,
    state_id: Option<StorageStateId>,
    target_generation: u64,
    target_state_id: Option<StorageStateId>,
) -> RevisionResult<()> {
    let intent = FullCopyIntent {
        generation,
        state_id,
        target_generation,
        target_state_id,
    };
    directory.write_marker(
        super::super::PUBLISH_FULL_COPY_INTENT_FILE,
        &intent.encode(),
    )
}

pub(in crate::live) fn recover_full_copy(
    directory: &PrivateDirectory,
    destination: &Path,
) -> RevisionResult<()> {
    let intent = match directory.read_marker(super::super::PUBLISH_FULL_COPY_INTENT_FILE) {
        Ok(bytes) => FullCopyIntent::decode(&bytes)?,
        Err(RevisionError::Io(error)) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let inspection = match inspect_canonical(destination) {
        Ok(inspection) => inspection,
        Err(RevisionError::Io(error))
            if error.kind() == io::ErrorKind::NotFound
                && intent.generation == 0
                && intent.state_id.is_none() =>
        {
            remove_private_file(directory, super::super::PUBLISH_FULL_COPY_INTENT_FILE)?;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    if inspection.generation == intent.target_generation
        && inspection.state_id == intent.target_state_id
    {
        write_publication_marker(directory, intent.target_generation, intent.target_state_id)?;
    } else if inspection.generation == intent.generation && inspection.state_id == intent.state_id {
        write_publication_marker(directory, intent.generation, intent.state_id)?;
    } else {
        return Err(RevisionError::Invalid(
            "full-copy publication residual does not match its recorded base or target".into(),
        ));
    }
    remove_private_file(directory, super::super::PUBLISH_FULL_COPY_INTENT_FILE)
}

fn inspect_canonical(destination: &Path) -> RevisionResult<crate::store::StoreInspection> {
    let canonical = open_nofollow(destination, false)?;
    let identity = validated_identity(&canonical, false)?;
    let inspection = RevisionStore::inspect(destination)?;
    if validated_identity(&canonical, false)? != identity {
        return Err(RevisionError::Invalid(
            "canonical project changed during full-copy recovery".into(),
        ));
    }
    let revalidated = open_nofollow(destination, false)?;
    if validated_identity(&revalidated, false)? != identity {
        return Err(RevisionError::Invalid(
            "canonical project changed after full-copy recovery inspection".into(),
        ));
    }
    Ok(inspection)
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
        && (publication_marker_matches(
            &PrivateDirectory::open(cache_directory)?,
            inspection.generation,
            inspection.state_id,
        ) || exchange_proof_matches(destination, inspection.generation, inspection.state_id)?);
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
