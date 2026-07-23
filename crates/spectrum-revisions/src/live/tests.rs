#![cfg(target_os = "linux")]

use std::{
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::unix::fs::{FileExt, MetadataExt, symlink},
};

use super::*;

fn bytes_file(path: &Path, bytes: &[u8]) -> File {
    fs::write(path, bytes).unwrap();
    open_private_writable(path).unwrap()
}

#[test]
fn mirror_diff_counts_growth_shrink_partial_tail_and_sparse_blocks_exactly() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let mirror = directory.path().join("mirror");

    let mut baseline = vec![0x11; WRITE_BLOCK_BYTES * 3 + 7];
    fs::write(&source, &baseline).unwrap();
    let mut mirror_file = bytes_file(&mirror, &baseline);
    baseline[WRITE_BLOCK_BYTES + 9] = 0x22;
    fs::write(&source, &baseline).unwrap();
    let changed = update_mirror(&source, &mut mirror_file).unwrap();
    assert_eq!(changed.written_bytes, WRITE_BLOCK_BYTES as u64);
    assert_eq!(fs::read(&mirror).unwrap(), baseline);

    baseline.extend([0x33; 123]);
    fs::write(&source, &baseline).unwrap();
    let grown = update_mirror(&source, &mut mirror_file).unwrap();
    assert_eq!(grown.written_bytes, 123);
    assert_eq!(fs::read(&mirror).unwrap(), baseline);

    baseline.truncate(WRITE_BLOCK_BYTES + 3);
    fs::write(&source, &baseline).unwrap();
    let shrunk = update_mirror(&source, &mut mirror_file).unwrap();
    assert_eq!(shrunk.written_bytes, 0);
    assert_eq!(mirror_file.metadata().unwrap().len(), baseline.len() as u64);

    baseline[WRITE_BLOCK_BYTES] = 0x44;
    fs::write(&source, &baseline).unwrap();
    let partial = update_mirror(&source, &mut mirror_file).unwrap();
    assert_eq!(partial.written_bytes, 3);
    assert_eq!(fs::read(&mirror).unwrap(), baseline);

    let sparse_len = 64 * 1024 * 1024;
    let sparse_source = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&source)
        .unwrap();
    sparse_source.set_len(sparse_len).unwrap();
    sparse_source
        .write_all_at(b"sparse-change", 32 * 1024 * 1024 + 17)
        .unwrap();
    mirror_file.set_len(sparse_len).unwrap();
    mirror_file.seek(SeekFrom::Start(0)).unwrap();
    let sparse = update_mirror(&source, &mut mirror_file).unwrap();
    assert_eq!(sparse.written_bytes, WRITE_BLOCK_BYTES as u64);
    assert_eq!(fs::read(&source).unwrap(), fs::read(&mirror).unwrap());
    assert!(mirror_file.metadata().unwrap().blocks() * 512 < sparse_len / 4);
}

#[test]
fn descriptor_validation_rejects_symlinks_hardlinks_and_replacements() {
    let directory = tempfile::tempdir().unwrap();
    let mirror = directory.path().join("mirror");
    let alias = directory.path().join("alias");
    fs::write(&mirror, b"mirror").unwrap();
    fs::hard_link(&mirror, &alias).unwrap();
    assert!(open_private_writable(&mirror).is_err());
    fs::remove_file(&alias).unwrap();

    let descriptor = open_private_writable(&mirror).unwrap();
    let identity = validated_identity(&descriptor, true).unwrap();
    let replacement = directory.path().join("replacement");
    fs::write(&replacement, b"replacement").unwrap();
    fs::rename(&replacement, &mirror).unwrap();
    assert!(validate_named_identity(&mirror, identity, true).is_err());

    let symlink_path = directory.path().join("symlink");
    symlink(&mirror, &symlink_path).unwrap();
    assert!(open_nofollow(&symlink_path, false).is_err());
}

#[test]
fn every_incremental_crash_phase_leaves_a_complete_old_or_new_project() {
    let old = vec![0x11; 128 * 1024];
    let mut new = old.clone();
    new[WRITE_BLOCK_BYTES + 3] = 0x22;
    let old_state = Some([0x5a; 16]);
    let new_state = Some([0x6b; 16]);
    for fault in [
        PublishFault::BackupLinked,
        PublishFault::MarkerRemoved,
        PublishFault::MirrorSynced,
        PublishFault::CanonicalRenamed,
        PublishFault::BackupRenamed,
        PublishFault::MirrorResynced,
        PublishFault::MarkerCreated,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("cache");
        fs::create_dir(&cache).unwrap();
        let destination = directory.path().join("project");
        let source = cache.join("source");
        let mirror = cache.join(PUBLISH_MIRROR_FILE);
        let ready = cache.join(PUBLISH_MIRROR_READY_FILE);
        fs::write(&destination, &old).unwrap();
        fs::write(&source, &new).unwrap();
        fs::write(&mirror, &old).unwrap();
        write_ready_marker(&ready, 1, old_state).unwrap();
        let canonical = open_nofollow(&destination, false).unwrap();
        let metadata = canonical.metadata().unwrap();
        let base = PublishBase {
            identity: Some(file_identity(&metadata)),
            permissions: Some(metadata.permissions()),
            canonical: Some(canonical),
            generation: 1,
            state_id: old_state,
        };

        PUBLISH_FAULT.set(Some(fault));
        let result = incremental_publish(&source, &destination, &cache, &base, 2, new_state);
        PUBLISH_FAULT.set(None);
        assert!(result.is_err(), "{fault:?} did not inject a failure");
        let visible = fs::read(&destination).unwrap();
        assert!(
            visible == old || visible == new,
            "{fault:?} exposed a torn project"
        );

        let visible_is_new = visible == new;
        let _ = fs::remove_file(cache.join(PUBLISH_BACKUP_FILE));
        let _ = fs::remove_file(&mirror);
        let _ = fs::remove_file(&ready);
        seed_incremental_mirror(
            &destination,
            &cache,
            if visible_is_new { 2 } else { 1 },
            if visible_is_new { new_state } else { old_state },
        );
        let canonical = open_nofollow(&destination, false).unwrap();
        let metadata = canonical.metadata().unwrap();
        let recovered_base = PublishBase {
            identity: Some(file_identity(&metadata)),
            permissions: Some(metadata.permissions()),
            canonical: Some(canonical),
            generation: if visible_is_new { 2 } else { 1 },
            state_id: if visible_is_new { new_state } else { old_state },
        };
        incremental_publish(&source, &destination, &cache, &recovered_base, 2, new_state)
            .unwrap()
            .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), new);
        assert!(ready_marker_matches(&ready, 2, new_state));
        assert_eq!(fs::read(&mirror).unwrap(), new);
    }
}

#[test]
fn failed_seed_never_marks_a_partial_mirror_ready() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("project");
    let cache = directory.path().join("cache");
    fs::create_dir(&cache).unwrap();
    fs::write(&destination, vec![0x33; 64 * 1024]).unwrap();
    PUBLISH_FAULT.set(Some(PublishFault::SeedMirrorCreated));
    seed_incremental_mirror(&destination, &cache, 1, Some([0x44; 16]));
    PUBLISH_FAULT.set(None);
    assert!(!cache.join(PUBLISH_MIRROR_READY_FILE).exists());
    assert!(!cache.join(PUBLISH_MIRROR_FILE).exists());
    seed_incremental_mirror(&destination, &cache, 1, Some([0x44; 16]));
    assert!(ready_marker_matches(
        &cache.join(PUBLISH_MIRROR_READY_FILE),
        1,
        Some([0x44; 16])
    ));
    assert_eq!(
        fs::read(cache.join(PUBLISH_MIRROR_FILE)).unwrap(),
        fs::read(destination).unwrap()
    );
}
