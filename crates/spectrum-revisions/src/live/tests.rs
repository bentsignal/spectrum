#![cfg(target_os = "linux")]

use std::{
    fs::{self, File, OpenOptions},
    os::unix::{
        fs::{FileExt, MetadataExt, PermissionsExt, symlink},
        process::ExitStatusExt,
    },
    process::Command,
};

use super::linux_io::{
    ExchangeIntent, apply_checkpoint_delta, inspect_named_checkpoint, publication_marker_matches,
    ready_marker_matches,
};
use super::*;
use crate::{Actor, ActorKind, Asset, Payload};

fn bytes_file(path: &Path, bytes: &[u8]) -> File {
    fs::write(path, bytes).unwrap();
    open_nofollow(path, true).unwrap()
}

fn private_directory(path: &Path) -> PrivateDirectory {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    PrivateDirectory::open(path).unwrap()
}

#[test]
fn mirror_delta_counts_growth_removed_tail_and_written_blocks_exactly() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let mirror = directory.path().join("mirror");

    let mut baseline = vec![0x11; WRITE_BLOCK_BYTES * 3 + 7];
    fs::write(&source, &baseline).unwrap();
    let mut mirror_file = bytes_file(&mirror, &baseline);
    baseline[WRITE_BLOCK_BYTES + 9] = 0x22;
    fs::write(&source, &baseline).unwrap();
    let changed = apply_checkpoint_delta(&source, &mirror_file).unwrap();
    assert_eq!(changed.changed_bytes, WRITE_BLOCK_BYTES as u64);
    assert_eq!(changed.written_bytes, WRITE_BLOCK_BYTES as u64);

    fs::write(&mirror, &baseline).unwrap();
    baseline.extend([0x33; 123]);
    fs::write(&source, &baseline).unwrap();
    mirror_file = open_nofollow(&mirror, true).unwrap();
    let grown = apply_checkpoint_delta(&source, &mirror_file).unwrap();
    assert_eq!(grown.changed_bytes, 123);

    fs::write(&mirror, &baseline).unwrap();
    let grown_len = baseline.len();
    baseline.truncate(WRITE_BLOCK_BYTES + 3);
    fs::write(&source, &baseline).unwrap();
    mirror_file = open_nofollow(&mirror, true).unwrap();
    let shrunk = apply_checkpoint_delta(&source, &mirror_file).unwrap();
    assert_eq!(
        shrunk.changed_bytes,
        u64::try_from(grown_len - baseline.len()).unwrap()
    );
    assert_eq!(shrunk.written_bytes, 0);

    fs::write(&mirror, &baseline).unwrap();
    baseline[WRITE_BLOCK_BYTES] = 0x44;
    fs::write(&source, &baseline).unwrap();
    mirror_file = open_nofollow(&mirror, true).unwrap();
    let partial = apply_checkpoint_delta(&source, &mirror_file).unwrap();
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
    mirror_file = open_nofollow(&mirror, true).unwrap();
    let sparse = apply_checkpoint_delta(&source, &mirror_file).unwrap();
    assert_eq!(sparse.changed_bytes, WRITE_BLOCK_BYTES as u64);
    assert_eq!(sparse.written_bytes, WRITE_BLOCK_BYTES as u64);
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
fn exchange_intent_decoder_rejects_noncanonical_and_impossible_records() {
    let intent = ExchangeIntent {
        canonical_identity: FileIdentity {
            device: 1,
            inode: 2,
        },
        candidate_identity: FileIdentity {
            device: 1,
            inode: 3,
        },
        generation: 7,
        state_id: None,
        target_generation: 8,
        target_state_id: None,
    };
    let mut absent_state_with_payload = intent.encode();
    absent_state_with_payload[57] = 1;
    assert!(ExchangeIntent::decode(&absent_state_with_payload).is_err());
    let mut absent_target_with_payload = intent.encode();
    absent_target_with_payload[74] = 1;
    assert!(ExchangeIntent::decode(&absent_target_with_payload).is_err());

    let mut equal_identities = intent;
    equal_identities.candidate_identity = equal_identities.canonical_identity;
    assert!(ExchangeIntent::decode(&equal_identities.encode()).is_err());

    let mut nonadvancing = intent;
    nonadvancing.target_generation = nonadvancing.generation;
    assert!(ExchangeIntent::decode(&nonadvancing.encode()).is_err());
    assert_eq!(
        ExchangeIntent::decode(&intent.encode())
            .unwrap()
            .target_generation,
        8
    );
}

#[test]
fn recovery_inspection_rejects_an_ordinary_path_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("canonical.prism");
    let replacement = directory.path().join("replacement.prism");
    let project = |application_id: &str| NewProject {
        application_id: application_id.into(),
        application_version: "1.0.0".into(),
        actor: Actor {
            id: format!("{application_id}:actor"),
            display_name: "Recovery test".into(),
            kind: ActorKind::System,
        },
        session_id: SessionId::new(),
        root_label: Some("Created".into()),
        track_kind: "test.document".into(),
        track_label: "Document".into(),
        initial_snapshots: vec![Payload::new(
            crate::Encoding::new("test.snapshot", 1),
            b"root".to_vec(),
        )],
        assets: Vec::new(),
    };
    let (original, _) =
        RevisionStore::create(&canonical, project("spectrum.recovery-original")).unwrap();
    drop(original);
    let (replacement_store, _) =
        RevisionStore::create(&replacement, project("spectrum.recovery-replacement")).unwrap();
    drop(replacement_store);
    let held = open_nofollow(&canonical, false).unwrap();
    let identity = validated_identity(&held, false).unwrap();

    fs::rename(&replacement, &canonical).unwrap();
    assert!(inspect_named_checkpoint(&canonical, &held, identity, false).is_err());
}

#[test]
fn linked_private_slot_is_rejected_before_delta_or_exchange() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let mirror = directory.path().join("mirror");
    let external = directory.path().join("external");
    fs::write(&source, b"new candidate bytes").unwrap();
    fs::write(&mirror, b"old mirror bytes").unwrap();
    fs::hard_link(&mirror, &external).unwrap();
    let descriptor = open_nofollow(&mirror, true).unwrap();
    assert!(validated_identity(&descriptor, true).is_err());
    assert_eq!(fs::read(&external).unwrap(), b"old mirror bytes");
    assert_eq!(fs::read(&mirror).unwrap(), b"old mirror bytes");
}

#[test]
fn atomic_marker_replacement_never_follows_a_symlink() {
    let directory = tempfile::tempdir().unwrap();
    let private = private_directory(directory.path());
    let marker = directory.path().join(PUBLISH_MIRROR_READY_FILE);
    let victim = directory.path().join("victim");
    fs::write(&victim, b"untouched").unwrap();
    symlink(&victim, &marker).unwrap();

    write_ready_marker(&private, 7, Some([0x42; 16])).unwrap();
    assert_eq!(fs::read(&victim).unwrap(), b"untouched");
    assert!(marker.symlink_metadata().unwrap().file_type().is_file());
    assert!(ready_marker_matches(&private, 7, Some([0x42; 16])));
}

#[test]
fn private_directory_rejects_shared_modes_and_symlink_entry_points() {
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    fs::create_dir(&cache).unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(PrivateDirectory::open(&cache).is_err());
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    let alias = root.path().join("cache-alias");
    symlink(&cache, &alias).unwrap();
    assert!(PrivateDirectory::open(&alias).is_err());
    assert!(PrivateDirectory::open(&cache).is_ok());
}

#[test]
fn alternating_slot_two_generations_behind_publishes_only_dirty_blocks() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let (mut live, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        NewProject {
            application_id: "spectrum.exchange-test".into(),
            application_version: "1.0.0".into(),
            actor: Actor {
                id: "exchange-test".into(),
                display_name: "Exchange test".into(),
                kind: ActorKind::System,
            },
            session_id: SessionId::new(),
            root_label: Some("Created".into()),
            track_kind: "test.document".into(),
            track_label: "Document".into(),
            initial_snapshots: vec![Payload::new(
                crate::Encoding::new("test.snapshot", 1),
                vec![0x55; 64 * 1024],
            )],
            assets: vec![Asset::new(
                "application/x-baseline",
                vec![0x44; 1024 * 1024],
            )],
        },
    )
    .unwrap();
    let project_cache = cache.join(info.project_id.to_string());

    live.mutate(|store| store.put_asset("application/x-first", b"first"))
        .unwrap();
    let first = live.last_publish_stats();
    assert_eq!(first.strategy, PublishStrategy::PageDiffExchange);
    assert!(first.incremental);
    assert!(first.written_bytes <= 64 * 1024);
    let canonical_after_first = RevisionStore::inspect(&canonical).unwrap();
    let slot_after_first =
        RevisionStore::inspect(&project_cache.join(PUBLISH_MIRROR_FILE)).unwrap();
    assert_eq!(
        canonical_after_first.generation,
        slot_after_first.generation + 1
    );

    live.mutate(|store| store.put_asset("application/x-second", b"second"))
        .unwrap();
    let second = live.last_publish_stats();
    assert_eq!(second.strategy, PublishStrategy::PageDiffExchange);
    assert!(second.incremental);
    assert!(second.written_bytes <= 64 * 1024);
    assert_eq!(
        RevisionStore::inspect(&canonical).unwrap().generation,
        canonical_after_first.generation + 1
    );
}

#[test]
fn bulk_growth_catches_up_the_alternate_slot_before_the_next_small_edit() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let (mut live, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        NewProject {
            application_id: "spectrum.bulk-catch-up-test".into(),
            application_version: "1.0.0".into(),
            actor: Actor {
                id: "bulk-catch-up-test".into(),
                display_name: "Bulk catch-up test".into(),
                kind: ActorKind::System,
            },
            session_id: SessionId::new(),
            root_label: Some("Created".into()),
            track_kind: "test.document".into(),
            track_label: "Document".into(),
            initial_snapshots: vec![Payload::new(
                crate::Encoding::new("test.snapshot", 1),
                b"root".to_vec(),
            )],
            assets: Vec::new(),
        },
    )
    .unwrap();
    let project_cache = cache.join(info.project_id.to_string());
    let mirror = project_cache.join(PUBLISH_MIRROR_FILE);

    live.mutate(|store| store.put_asset("application/x-bulk", &vec![0x5a; 2 * 1024 * 1024]))
        .unwrap();
    let canonical_after_bulk = RevisionStore::inspect(&canonical).unwrap();
    let slot_after_bulk = RevisionStore::inspect(&mirror).unwrap();
    assert_eq!(slot_after_bulk.generation, canonical_after_bulk.generation);
    assert_eq!(slot_after_bulk.state_id, canonical_after_bulk.state_id);

    live.mutate(|store| store.put_asset("application/x-small", b"small"))
        .unwrap();
    let small = live.last_publish_stats();
    assert_eq!(small.strategy, PublishStrategy::PageDiffExchange);
    assert!(small.incremental);
    assert!(
        small.written_bytes <= 64 * 1024,
        "small edit after bulk growth wrote {} bytes",
        small.written_bytes
    );
}

#[test]
fn committed_full_copy_crash_recovers_for_an_already_open_survivor() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let alias = directory.path().join("external-alias.lumen");
    let cache = directory.path().join("cache");
    let (mut live, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        NewProject {
            application_id: "spectrum.hardlink-fallback-test".into(),
            application_version: "1.0.0".into(),
            actor: Actor {
                id: "hardlink-fallback-test".into(),
                display_name: "Hardlink fallback test".into(),
                kind: ActorKind::System,
            },
            session_id: SessionId::new(),
            root_label: Some("Created".into()),
            track_kind: "test.document".into(),
            track_label: "Document".into(),
            initial_snapshots: vec![Payload::new(
                crate::Encoding::new("test.snapshot", 1),
                b"root".to_vec(),
            )],
            assets: Vec::new(),
        },
    )
    .unwrap();
    fs::set_permissions(&canonical, fs::Permissions::from_mode(0o640)).unwrap();
    fs::hard_link(&canonical, &alias).unwrap();
    let alias_bytes = fs::read(&alias).unwrap();
    let alias_mode = alias.metadata().unwrap().permissions().mode();
    let base_generation = live.store().generation().unwrap();

    let status = Command::new(std::env::current_exe().unwrap())
        .arg("publication_crash_child")
        .arg("--nocapture")
        .env("SPECTRUM_CRASH_CANONICAL", &canonical)
        .env("SPECTRUM_CRASH_CACHE", &cache)
        .env("SPECTRUM_CRASH_FAULT", "full-copy-published")
        .env("SPECTRUM_CRASH_MODE", "exit")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    assert!(RevisionStore::inspect(&canonical).unwrap().generation > base_generation);
    assert!(
        cache
            .join(info.project_id.to_string())
            .join(PUBLISH_FULL_COPY_INTENT_FILE)
            .exists()
    );
    assert_eq!(fs::read(&alias).unwrap(), alias_bytes);
    assert_eq!(alias.metadata().unwrap().permissions().mode(), alias_mode);
    assert_eq!(canonical.metadata().unwrap().nlink(), 1);

    live.mutate(|store| store.put_asset("application/x-after-fallback", b"next edit"))
        .unwrap();
    assert!(live.pending_publish_error().is_none());
    let child_asset = Asset::new(
        "application/x-crash-child",
        b"survives abrupt death".to_vec(),
    );
    assert!(live.store().asset(child_asset.id).unwrap().is_some());
    assert_eq!(
        live.last_publish_stats().strategy,
        PublishStrategy::PageDiffExchange
    );
    live.publish().unwrap();
    assert_eq!(live.last_publish_stats(), PublishStats::default());
    let verified = RevisionStore::open_read_only(&canonical).unwrap();
    assert!(verified.asset(child_asset.id).unwrap().is_some());
    assert!(
        verified
            .asset(Asset::new("application/x-after-fallback", b"next edit".to_vec()).id)
            .unwrap()
            .is_some()
    );
    assert_eq!(fs::read(&alias).unwrap(), alias_bytes);
    assert_eq!(alias.metadata().unwrap().permissions().mode(), alias_mode);
}

#[test]
fn hardlink_created_at_exchange_is_discarded_from_private_recovery_without_alias_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let alias = directory.path().join("exchange-race-alias.lumen");
    let cache = directory.path().join("cache");
    let (mut live, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        NewProject {
            application_id: "spectrum.hardlink-race-test".into(),
            application_version: "1.0.0".into(),
            actor: Actor {
                id: "hardlink-race-test".into(),
                display_name: "Hardlink race test".into(),
                kind: ActorKind::System,
            },
            session_id: SessionId::new(),
            root_label: Some("Created".into()),
            track_kind: "test.document".into(),
            track_label: "Document".into(),
            initial_snapshots: vec![Payload::new(
                crate::Encoding::new("test.snapshot", 1),
                vec![0x44; 128 * 1024],
            )],
            assets: Vec::new(),
        },
    )
    .unwrap();
    fs::set_permissions(&canonical, fs::Permissions::from_mode(0o640)).unwrap();
    let alias_bytes = fs::read(&canonical).unwrap();
    let alias_mode = canonical.metadata().unwrap().permissions().mode();
    let base_generation = live.store().generation().unwrap();
    let project_cache = cache.join(info.project_id.to_string());

    PUBLISH_HARDLINK_ALIAS.with(|hook| hook.replace(Some(alias.clone())));
    PUBLISH_FAULT.set(Some(PublishFault::Exchanged));
    live.mutate(|store| store.put_asset("application/x-raced", b"committed"))
        .unwrap();
    assert!(live.pending_publish_error().is_some());
    PUBLISH_FAULT.set(None);
    PUBLISH_HARDLINK_ALIAS.with(|hook| hook.replace(None));

    assert!(RevisionStore::inspect(&canonical).unwrap().generation > base_generation);
    assert_ne!(fs::read(&canonical).unwrap(), alias_bytes);
    assert_eq!(fs::read(&alias).unwrap(), alias_bytes);
    assert_eq!(alias.metadata().unwrap().permissions().mode(), alias_mode);
    assert_eq!(alias.metadata().unwrap().nlink(), 2);
    assert!(project_cache.join(PUBLISH_EXCHANGE_INTENT_FILE).exists());
    live.publish().unwrap();
    assert!(live.pending_publish_error().is_none());
    assert_eq!(live.last_publish_stats(), PublishStats::default());
    assert!(!project_cache.join(PUBLISH_EXCHANGE_INTENT_FILE).exists());
    assert!(!project_cache.join(PUBLISH_MIRROR_FILE).exists());
    assert_eq!(alias.metadata().unwrap().nlink(), 1);
    assert_eq!(fs::read(&alias).unwrap(), alias_bytes);
    assert_eq!(alias.metadata().unwrap().permissions().mode(), alias_mode);
    drop(live);

    let mut recovered = LiveRevisionStore::open(&canonical, &cache).unwrap();
    assert!(!project_cache.join(PUBLISH_EXCHANGE_INTENT_FILE).exists());
    assert!(!project_cache.join(PUBLISH_MIRROR_FILE).exists());
    assert_eq!(alias.metadata().unwrap().nlink(), 1);
    assert_eq!(fs::read(&alias).unwrap(), alias_bytes);
    assert_eq!(alias.metadata().unwrap().permissions().mode(), alias_mode);
    recovered
        .mutate(|store| store.put_asset("application/x-after-race", b"next edit"))
        .unwrap();
    assert_eq!(
        recovered.last_publish_stats().strategy,
        PublishStrategy::FullCopy
    );
    assert_eq!(fs::read(&alias).unwrap(), alias_bytes);
    assert_eq!(alias.metadata().unwrap().permissions().mode(), alias_mode);
}

#[test]
fn child_crash_after_linked_slot_unlink_recovers_without_the_slot() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let alias = directory.path().join("child-race-alias.lumen");
    let cache = directory.path().join("cache");
    let (live, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        NewProject {
            application_id: "spectrum.hardlink-child-crash-test".into(),
            application_version: "1.0.0".into(),
            actor: Actor {
                id: "hardlink-child-crash-test".into(),
                display_name: "Hardlink child crash test".into(),
                kind: ActorKind::System,
            },
            session_id: SessionId::new(),
            root_label: Some("Created".into()),
            track_kind: "test.document".into(),
            track_label: "Document".into(),
            initial_snapshots: vec![Payload::new(
                crate::Encoding::new("test.snapshot", 1),
                vec![0x55; 128 * 1024],
            )],
            assets: Vec::new(),
        },
    )
    .unwrap();
    fs::set_permissions(&canonical, fs::Permissions::from_mode(0o640)).unwrap();
    let alias_bytes = fs::read(&canonical).unwrap();
    let alias_mode = canonical.metadata().unwrap().permissions().mode();
    let base_generation = live.store().generation().unwrap();
    let project_cache = cache.join(info.project_id.to_string());
    drop(live);

    let status = Command::new(std::env::current_exe().unwrap())
        .arg("publication_crash_child")
        .arg("--nocapture")
        .env("SPECTRUM_CRASH_CANONICAL", &canonical)
        .env("SPECTRUM_CRASH_CACHE", &cache)
        .env("SPECTRUM_CRASH_HARDLINK_ALIAS", &alias)
        .env(
            "SPECTRUM_CRASH_FAULT",
            fault_name(PublishFault::LinkedSlotUnlinked),
        )
        .env("SPECTRUM_CRASH_MODE", "kill")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert!(RevisionStore::inspect(&canonical).unwrap().generation > base_generation);
    assert!(project_cache.join(PUBLISH_EXCHANGE_INTENT_FILE).exists());
    assert!(!project_cache.join(PUBLISH_MIRROR_FILE).exists());
    assert_eq!(fs::read(&alias).unwrap(), alias_bytes);
    assert_eq!(alias.metadata().unwrap().permissions().mode(), alias_mode);

    let child_asset = Asset::new(
        "application/x-crash-child",
        b"survives abrupt death".to_vec(),
    );
    let mut recovered = LiveRevisionStore::open(&canonical, &cache).unwrap();
    assert!(!project_cache.join(PUBLISH_EXCHANGE_INTENT_FILE).exists());
    assert!(
        recovered.store().asset(child_asset.id).unwrap().is_some(),
        "the committed child mutation was not recovered"
    );
    recovered
        .mutate(|store| store.put_asset("application/x-after-missing-slot", b"next edit"))
        .unwrap();
    assert_eq!(
        recovered.last_publish_stats().strategy,
        PublishStrategy::FullCopy
    );
    assert_eq!(fs::read(&alias).unwrap(), alias_bytes);
    assert_eq!(alias.metadata().unwrap().permissions().mode(), alias_mode);
}

#[test]
fn private_lock_probe_child() {
    let Ok(cache) = std::env::var("SPECTRUM_PRIVATE_LOCK_CACHE") else {
        return;
    };
    use fs2::FileExt as _;

    let directory = PrivateDirectory::open(Path::new(&cache)).unwrap();
    let lock = directory.open_file(LOCK_FILE, true).unwrap();
    assert!(
        lock.try_lock_exclusive().is_err(),
        "cross-process private publish lock was not held"
    );
}

#[test]
fn private_publish_lock_blocks_same_process_and_child_process() {
    use fs2::FileExt as _;

    let directory = tempfile::tempdir().unwrap();
    let private = private_directory(directory.path());
    let lock_path = directory.path().join(LOCK_FILE);
    let held = lock_private(&lock_path).unwrap();
    let second = private.open_file(LOCK_FILE, true).unwrap();
    assert!(second.try_lock_exclusive().is_err());
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("private_lock_probe_child")
        .arg("--nocapture")
        .env("SPECTRUM_PRIVATE_LOCK_CACHE", directory.path())
        .status()
        .unwrap();
    assert!(status.success());
    drop(held);
    second.try_lock_exclusive().unwrap();
}

#[test]
fn publication_crash_child() {
    let Ok(canonical) = std::env::var("SPECTRUM_CRASH_CANONICAL") else {
        return;
    };
    let cache = std::env::var("SPECTRUM_CRASH_CACHE").unwrap();
    if let Ok(alias) = std::env::var("SPECTRUM_CRASH_HARDLINK_ALIAS") {
        PUBLISH_HARDLINK_ALIAS.with(|hook| hook.replace(Some(PathBuf::from(alias))));
    }
    let fault = parse_fault(&std::env::var("SPECTRUM_CRASH_FAULT").unwrap());
    let mode = match std::env::var("SPECTRUM_CRASH_MODE").unwrap().as_str() {
        "exit" => CrashMode::Exit,
        "abort" => CrashMode::Abort,
        "kill" => CrashMode::Kill,
        other => panic!("unknown crash mode {other}"),
    };
    let mut live = LiveRevisionStore::open(Path::new(&canonical), Path::new(&cache)).unwrap();
    if let Ok(opened) = std::env::var("SPECTRUM_CRASH_OPEN_SENTINEL") {
        fs::write(opened, b"store open").unwrap();
    }
    if let Ok(continue_sentinel) = std::env::var("SPECTRUM_CRASH_WAIT_FOR") {
        for _ in 0..1_000 {
            if Path::new(&continue_sentinel).exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            Path::new(&continue_sentinel).exists(),
            "timed out waiting to continue crash child"
        );
    }
    let prime_handled_failure = std::env::var("SPECTRUM_CRASH_PRIME_HANDLED_FAILURE").is_ok();
    if prime_handled_failure {
        PUBLISH_FAULT.set(Some(PublishFault::CandidateSynced));
        live.mutate(|store| {
            store.put_asset(
                "application/x-durable-prefix",
                b"acknowledged before the next mutation",
            )
        })
        .unwrap();
        assert!(live.pending_publish_error().is_some());
        PUBLISH_FAULT.set(None);
    }
    PUBLISH_CRASH_MODE.set(Some(mode));
    let arm_after_mutation =
        prime_handled_failure || std::env::var("SPECTRUM_CRASH_ARM_AFTER_MUTATION").is_ok();
    if arm_after_mutation {
        live.mutate(|store| {
            let result = store.put_asset("application/x-crash-child", b"survives abrupt death");
            if let Ok(sentinel) = std::env::var("SPECTRUM_CRASH_TAIL_SENTINEL") {
                fs::write(sentinel, b"tail mutation committed").unwrap();
            }
            PUBLISH_FAULT.set(Some(fault));
            result
        })
        .unwrap();
    } else {
        PUBLISH_FAULT.set(Some(fault));
        live.mutate(|store| store.put_asset("application/x-crash-child", b"survives abrupt death"))
            .unwrap();
    }
    panic!("publication fault {fault:?} did not terminate the child");
}

#[test]
fn every_incremental_crash_phase_recovers_actual_residual_state() {
    let faults = [
        PublishFault::SlotSealed,
        PublishFault::CandidateSynced,
        PublishFault::IntentCreated,
        PublishFault::PreExchangeValidated,
        PublishFault::Exchanged,
        PublishFault::SlotWritable,
        PublishFault::MarkerCreated,
        PublishFault::IntentRemoved,
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
        let base_generation = live.store().generation().unwrap();
        let project_cache = cache.join(info.project_id.to_string());
        let mut survivor = LiveRevisionStore::open(&canonical, &cache).unwrap();
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
        match mode {
            "exit" => assert_eq!(status.code(), Some(86), "{fault:?} did not reach _exit"),
            "abort" => assert_eq!(
                status.signal(),
                Some(libc::SIGABRT),
                "{fault:?} did not reach abort"
            ),
            "kill" => assert_eq!(
                status.signal(),
                Some(libc::SIGKILL),
                "{fault:?} did not reach SIGKILL"
            ),
            _ => unreachable!(),
        }
        let exchanged = matches!(
            fault,
            PublishFault::Exchanged
                | PublishFault::SlotWritable
                | PublishFault::MarkerCreated
                | PublishFault::IntentRemoved
        );
        assert_eq!(
            RevisionStore::inspect(&canonical).unwrap().generation > base_generation,
            exchanged,
            "{fault:?} left an unexpected canonical generation"
        );
        let intent_expected = matches!(
            fault,
            PublishFault::IntentCreated
                | PublishFault::PreExchangeValidated
                | PublishFault::Exchanged
                | PublishFault::SlotWritable
                | PublishFault::MarkerCreated
        );
        assert_eq!(
            project_cache.join(PUBLISH_EXCHANGE_INTENT_FILE).exists(),
            intent_expected,
            "{fault:?} left an unexpected intent residual"
        );
        assert_eq!(
            project_cache.join(PUBLISH_MIRROR_READY_FILE).exists(),
            matches!(
                fault,
                PublishFault::MarkerCreated | PublishFault::IntentRemoved
            ),
            "{fault:?} left an unexpected ready residual"
        );

        let child_asset = Asset::new(
            "application/x-crash-child",
            b"survives abrupt death".to_vec(),
        );
        let followup = format!("followup-{fault:?}");
        let followup_asset = Asset::new("application/x-followup", followup.as_bytes().to_vec());
        assert!(
            survivor.store().asset(child_asset.id).unwrap().is_some(),
            "{fault:?} lost the child mutation"
        );
        survivor
            .mutate(|store| store.put_asset("application/x-followup", followup.as_bytes()))
            .unwrap();
        assert!(
            survivor.pending_publish_error().is_none(),
            "{fault:?} left the already-open survivor unable to publish"
        );
        survivor.publish().unwrap();
        assert_eq!(survivor.last_publish_stats(), PublishStats::default());
        let survivor_inspection = RevisionStore::inspect(survivor.working_path()).unwrap();
        let canonical_inspection = RevisionStore::inspect(&canonical).unwrap();
        assert_eq!(
            canonical_inspection.generation,
            survivor_inspection.generation
        );
        assert_eq!(canonical_inspection.state_id, survivor_inspection.state_id);
        assert!(publication_marker_matches(
            &PrivateDirectory::open(&project_cache).unwrap(),
            canonical_inspection.generation,
            canonical_inspection.state_id,
        ));
        drop(survivor);

        let verified =
            LiveRevisionStore::open(&canonical, &directory.path().join("verification-cache"))
                .unwrap();
        assert!(verified.store().asset(child_asset.id).unwrap().is_some());
        assert!(verified.store().asset(followup_asset.id).unwrap().is_some());
    }
}

fn fault_name(fault: PublishFault) -> &'static str {
    match fault {
        PublishFault::SlotSealed => "slot-sealed",
        PublishFault::CandidateSynced => "candidate-synced",
        PublishFault::IntentCreated => "intent-created",
        PublishFault::PreExchangeValidated => "pre-exchange-validated",
        PublishFault::Exchanged => "exchanged",
        PublishFault::SlotWritable => "slot-writable",
        PublishFault::MarkerCreated => "marker-created",
        PublishFault::IntentRemoved => "intent-removed",
        PublishFault::LinkedSlotUnlinked => "linked-slot-unlinked",
        PublishFault::FullCopyPublished => "full-copy-published",
        PublishFault::SeedMirrorCreated => "seed-mirror-created",
        PublishFault::WorkingPoisonRenamed => "working-poison-renamed",
        PublishFault::WorkingPoisonSynced => "working-poison-synced",
        PublishFault::WorkingRecoveryMarkerRenamed => "working-recovery-marker-renamed",
        PublishFault::WorkingRecoveryMarkerSynced => "working-recovery-marker-synced",
        PublishFault::WorkingPoisonRemoved => "working-poison-removed",
        PublishFault::WorkingPoisonRemovalSynced => "working-poison-removal-synced",
    }
}

fn parse_fault(name: &str) -> PublishFault {
    match name {
        "slot-sealed" => PublishFault::SlotSealed,
        "candidate-synced" => PublishFault::CandidateSynced,
        "intent-created" => PublishFault::IntentCreated,
        "pre-exchange-validated" => PublishFault::PreExchangeValidated,
        "exchanged" => PublishFault::Exchanged,
        "slot-writable" => PublishFault::SlotWritable,
        "marker-created" => PublishFault::MarkerCreated,
        "intent-removed" => PublishFault::IntentRemoved,
        "linked-slot-unlinked" => PublishFault::LinkedSlotUnlinked,
        "full-copy-published" => PublishFault::FullCopyPublished,
        "seed-mirror-created" => PublishFault::SeedMirrorCreated,
        "working-poison-renamed" => PublishFault::WorkingPoisonRenamed,
        "working-poison-synced" => PublishFault::WorkingPoisonSynced,
        "working-recovery-marker-renamed" => PublishFault::WorkingRecoveryMarkerRenamed,
        "working-recovery-marker-synced" => PublishFault::WorkingRecoveryMarkerSynced,
        "working-poison-removed" => PublishFault::WorkingPoisonRemoved,
        "working-poison-removal-synced" => PublishFault::WorkingPoisonRemovalSynced,
        other => panic!("unknown publication fault {other}"),
    }
}

#[test]
fn failed_seed_never_marks_a_partial_mirror_ready() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("project");
    let cache = directory.path().join("cache");
    fs::create_dir(&cache).unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&destination, vec![0x33; 64 * 1024]).unwrap();
    PUBLISH_FAULT.set(Some(PublishFault::SeedMirrorCreated));
    seed_incremental_mirror(&destination, &cache, 1, Some([0x44; 16]));
    PUBLISH_FAULT.set(None);
    assert!(!cache.join(PUBLISH_MIRROR_READY_FILE).exists());
    assert!(!cache.join(PUBLISH_MIRROR_FILE).exists());
    seed_incremental_mirror(&destination, &cache, 1, Some([0x44; 16]));
    let private = PrivateDirectory::open(&cache).unwrap();
    assert!(ready_marker_matches(&private, 1, Some([0x44; 16])));
    assert_eq!(
        fs::read(cache.join(PUBLISH_MIRROR_FILE)).unwrap(),
        fs::read(destination).unwrap()
    );
}
