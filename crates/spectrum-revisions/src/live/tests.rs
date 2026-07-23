#![cfg(target_os = "linux")]

use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{FileExt, MetadataExt, symlink},
    process::Command,
};

use super::*;
use crate::{Actor, ActorKind, Asset, Payload};

fn bytes_file(path: &Path, bytes: &[u8]) -> File {
    fs::write(path, bytes).unwrap();
    open_nofollow(path, false).unwrap()
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
    let changed = compare_checkpoint(&source, &mut mirror_file).unwrap();
    assert_eq!(changed.changed_bytes, WRITE_BLOCK_BYTES as u64);
    assert_eq!(changed.written_bytes, 0);

    fs::write(&mirror, &baseline).unwrap();
    baseline.extend([0x33; 123]);
    fs::write(&source, &baseline).unwrap();
    mirror_file = open_nofollow(&mirror, false).unwrap();
    let grown = compare_checkpoint(&source, &mut mirror_file).unwrap();
    assert_eq!(grown.changed_bytes, 123);

    fs::write(&mirror, &baseline).unwrap();
    baseline.truncate(WRITE_BLOCK_BYTES + 3);
    fs::write(&source, &baseline).unwrap();
    mirror_file = open_nofollow(&mirror, false).unwrap();
    let shrunk = compare_checkpoint(&source, &mut mirror_file).unwrap();
    assert_eq!(shrunk.changed_bytes, 0);

    fs::write(&mirror, &baseline).unwrap();
    baseline[WRITE_BLOCK_BYTES] = 0x44;
    fs::write(&source, &baseline).unwrap();
    mirror_file = open_nofollow(&mirror, false).unwrap();
    let partial = compare_checkpoint(&source, &mut mirror_file).unwrap();
    assert_eq!(partial.changed_bytes, 3);

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
    let sparse_mirror = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&mirror)
        .unwrap();
    sparse_mirror.set_len(sparse_len).unwrap();
    mirror_file = open_nofollow(&mirror, false).unwrap();
    let sparse = compare_checkpoint(&source, &mut mirror_file).unwrap();
    assert_eq!(sparse.changed_bytes, WRITE_BLOCK_BYTES as u64);
    assert!(sparse_mirror.metadata().unwrap().blocks() * 512 < sparse_len / 4);
}

#[test]
fn descriptor_validation_rejects_symlinks_hardlinks_and_replacements() {
    let directory = tempfile::tempdir().unwrap();
    let mirror = directory.path().join("mirror");
    let alias = directory.path().join("alias");
    fs::write(&mirror, b"mirror").unwrap();
    fs::hard_link(&mirror, &alias).unwrap();
    let linked = open_nofollow(&mirror, false).unwrap();
    assert!(validated_identity(&linked, true).is_err());
    fs::remove_file(&alias).unwrap();

    let descriptor = open_nofollow(&mirror, false).unwrap();
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
fn hardlink_after_validation_never_receives_candidate_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let mirror = directory.path().join("mirror");
    let external = directory.path().join("external");
    fs::write(&source, b"new candidate bytes").unwrap();
    fs::write(&mirror, b"old mirror bytes").unwrap();
    let mut descriptor = open_nofollow(&mirror, false).unwrap();
    let identity = validated_identity(&descriptor, true).unwrap();

    fs::hard_link(&mirror, &external).unwrap();
    let stats = compare_checkpoint(&source, &mut descriptor).unwrap();
    assert!(stats.changed_bytes > 0);
    assert_eq!(stats.written_bytes, 0);
    assert_eq!(fs::read(&external).unwrap(), b"old mirror bytes");
    assert!(validate_named_identity(&mirror, identity, true).is_err());
}

#[test]
fn atomic_marker_replacement_never_follows_a_symlink() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("ready");
    let victim = directory.path().join("victim");
    fs::write(&victim, b"untouched").unwrap();
    symlink(&victim, &marker).unwrap();

    write_ready_marker(&marker, 7, Some([0x42; 16])).unwrap();
    assert_eq!(fs::read(&victim).unwrap(), b"untouched");
    assert!(marker.symlink_metadata().unwrap().file_type().is_file());
    assert!(ready_marker_matches(&marker, 7, Some([0x42; 16])));
}

#[test]
fn publication_crash_child() {
    let Ok(canonical) = std::env::var("SPECTRUM_CRASH_CANONICAL") else {
        return;
    };
    let cache = std::env::var("SPECTRUM_CRASH_CACHE").unwrap();
    let fault = parse_fault(&std::env::var("SPECTRUM_CRASH_FAULT").unwrap());
    let mode = match std::env::var("SPECTRUM_CRASH_MODE").unwrap().as_str() {
        "exit" => CrashMode::Exit,
        "abort" => CrashMode::Abort,
        "kill" => CrashMode::Kill,
        other => panic!("unknown crash mode {other}"),
    };
    let mut live = LiveRevisionStore::open(Path::new(&canonical), Path::new(&cache)).unwrap();
    PUBLISH_FAULT.set(Some(fault));
    PUBLISH_CRASH_MODE.set(Some(mode));
    live.mutate(|store| store.put_asset("application/x-crash-child", b"survives abrupt death"))
        .unwrap();
    panic!("publication fault {fault:?} did not terminate the child");
}

#[test]
fn every_incremental_crash_phase_recovers_actual_residual_state() {
    let faults = [
        PublishFault::MirrorSynced,
        PublishFault::BackupLinked,
        PublishFault::MarkerRemoved,
        PublishFault::CanonicalRenamed,
        PublishFault::BackupRenamed,
        PublishFault::MirrorResynced,
        PublishFault::MarkerCreated,
    ];
    for (index, fault) in faults.into_iter().enumerate() {
        let directory = tempfile::tempdir().unwrap();
        let canonical = directory.path().join("project.lumen");
        let cache = directory.path().join("cache");
        let session = SessionId::new();
        let (live, info) = LiveRevisionStore::create(
            &canonical,
            &cache,
            NewProject {
                application_id: "spectrum.crash-test".into(),
                application_version: "1.0.0".into(),
                actor: Actor {
                    id: "crash-test".into(),
                    display_name: "Crash test".into(),
                    kind: ActorKind::System,
                },
                session_id: session,
                root_label: Some("Created".into()),
                track_kind: "test.document".into(),
                track_label: "Document".into(),
                initial_snapshots: vec![Payload::new(
                    crate::Encoding::new("test.snapshot", 1),
                    vec![0x55; 128 * 1024],
                )],
                assets: vec![Asset::new(
                    "application/x-baseline",
                    vec![0x44; 1024 * 1024],
                )],
            },
        )
        .unwrap();
        let project_cache = cache.join(info.project_id.to_string());
        if clone_unnamed(live.working_path(), &project_cache).is_err() {
            return;
        }
        drop(live);

        let mode = ["exit", "abort", "kill"][index % 3];
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("publication_crash_child")
            .arg("--nocapture")
            .env("SPECTRUM_CRASH_CANONICAL", &canonical)
            .env("SPECTRUM_CRASH_CACHE", &cache)
            .env("SPECTRUM_CRASH_FAULT", fault_name(fault))
            .env("SPECTRUM_CRASH_MODE", mode)
            .status()
            .unwrap();
        assert!(!status.success(), "{fault:?} child unexpectedly succeeded");

        let child_asset = Asset::new(
            "application/x-crash-child",
            b"survives abrupt death".to_vec(),
        );
        let followup = format!("followup-{fault:?}");
        let followup_asset = Asset::new("application/x-followup", followup.as_bytes().to_vec());
        let mut recovered = LiveRevisionStore::open(&canonical, &cache).unwrap();
        assert!(
            recovered.store().asset(child_asset.id).unwrap().is_some(),
            "{fault:?} lost the child mutation"
        );
        recovered
            .mutate(|store| store.put_asset("application/x-followup", followup.as_bytes()))
            .unwrap();
        drop(recovered);

        let verified =
            LiveRevisionStore::open(&canonical, &directory.path().join("verification-cache"))
                .unwrap();
        assert!(verified.store().asset(child_asset.id).unwrap().is_some());
        assert!(verified.store().asset(followup_asset.id).unwrap().is_some());
    }
}

fn fault_name(fault: PublishFault) -> &'static str {
    match fault {
        PublishFault::BackupLinked => "backup-linked",
        PublishFault::MarkerRemoved => "marker-removed",
        PublishFault::MirrorSynced => "staged-links",
        PublishFault::CanonicalRenamed => "canonical-renamed",
        PublishFault::BackupRenamed => "mirror-renamed",
        PublishFault::MirrorResynced => "backup-removed",
        PublishFault::MarkerCreated => "marker-created",
        PublishFault::SeedMirrorCreated => "seed-mirror-created",
    }
}

fn parse_fault(name: &str) -> PublishFault {
    match name {
        "backup-linked" => PublishFault::BackupLinked,
        "marker-removed" => PublishFault::MarkerRemoved,
        "staged-links" => PublishFault::MirrorSynced,
        "canonical-renamed" => PublishFault::CanonicalRenamed,
        "mirror-renamed" => PublishFault::BackupRenamed,
        "backup-removed" => PublishFault::MirrorResynced,
        "marker-created" => PublishFault::MarkerCreated,
        "seed-mirror-created" => PublishFault::SeedMirrorCreated,
        other => panic!("unknown publication fault {other}"),
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
