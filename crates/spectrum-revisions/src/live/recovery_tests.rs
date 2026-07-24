use std::{
    os::unix::{fs::MetadataExt as _, process::ExitStatusExt as _},
    process::Command,
};

use super::linux_io::working_recovery_marker_matches;
use super::*;
use crate::{Actor, ActorKind, Asset, Payload};

fn project(application_id: &str) -> NewProject {
    NewProject {
        application_id: application_id.into(),
        application_version: "1.0.0".into(),
        actor: Actor {
            id: application_id.into(),
            display_name: "Working recovery test".into(),
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
    }
}

#[test]
fn initial_create_publication_is_one_shot_and_cannot_recreate_a_deleted_project() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let (live, _) =
        LiveRevisionStore::create(&canonical, &cache, project("spectrum.initial-publication"))
            .unwrap();
    assert!(!live.initial_publish_pending.get());
    assert!(canonical.is_file());

    std::fs::remove_file(&canonical).unwrap();
    assert!(live.publish().is_err());
    assert!(!canonical.exists());
}

#[test]
fn create_rejects_an_existing_zero_length_destination() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    std::fs::write(&canonical, []).unwrap();

    assert!(
        LiveRevisionStore::create(
            &canonical,
            &cache,
            project("spectrum.zero-length-destination"),
        )
        .is_err()
    );
    assert_eq!(std::fs::metadata(&canonical).unwrap().len(), 0);
    assert!(!cache.exists());
}

#[test]
fn initial_publication_never_replaces_a_destination_raced_in_after_validation() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let raced_bytes = b"concurrent creator owns this destination".to_vec();

    INITIAL_PUBLISH_RACE.with(|race| race.replace(Some(raced_bytes.clone())));
    let created = LiveRevisionStore::create(
        &canonical,
        &cache,
        project("spectrum.initial-publication-race"),
    );
    INITIAL_PUBLISH_RACE.with(|race| race.replace(None));

    assert!(created.is_err());
    assert_eq!(std::fs::read(&canonical).unwrap(), raced_bytes);
}

#[test]
fn initial_publication_crash_child() {
    let Ok(canonical) = std::env::var("SPECTRUM_INITIAL_CRASH_CANONICAL") else {
        return;
    };
    let cache = std::env::var("SPECTRUM_INITIAL_CRASH_CACHE").unwrap();
    PUBLISH_FAULT.set(Some(PublishFault::FullCopyPublished));
    PUBLISH_CRASH_MODE.set(Some(CrashMode::Kill));
    LiveRevisionStore::create(
        std::path::Path::new(&canonical),
        std::path::Path::new(&cache),
        project("spectrum.initial-publication-crash-child"),
    )
    .unwrap();
    panic!("initial publication fault did not terminate the child");
}

#[test]
fn initial_no_replace_publication_crash_leaves_one_valid_canonical_name() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");

    let status = Command::new(std::env::current_exe().unwrap())
        .arg("initial_publication_crash_child")
        .arg("--nocapture")
        .env("SPECTRUM_INITIAL_CRASH_CANONICAL", &canonical)
        .env("SPECTRUM_INITIAL_CRASH_CACHE", &cache)
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    let metadata = canonical.metadata().unwrap();
    assert_eq!(metadata.nlink(), 1);
    let inspection = RevisionStore::inspect(&canonical).unwrap();
    let temporary_prefix = ".project.lumen.spectrum-publish-";
    assert!(std::fs::read_dir(directory.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(temporary_prefix)
    }));

    let reopened = LiveRevisionStore::open(&canonical, &cache).unwrap();
    assert_eq!(
        reopened.store().generation().unwrap(),
        inspection.generation
    );
    assert_eq!(reopened.store().state_id().unwrap(), inspection.state_id);
}

#[test]
fn failed_initial_publication_reopens_through_durable_recovery_proof() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    PUBLISH_FAULT.set(Some(PublishFault::FullCopyPublished));
    let failed = LiveRevisionStore::create(
        &canonical,
        &cache,
        project("spectrum.initial-publication-retry"),
    );
    PUBLISH_FAULT.set(None);
    assert!(failed.is_err());
    assert!(canonical.is_file());

    let project_id = RevisionStore::inspect(&canonical).unwrap().info.project_id;
    let project_cache = cache.join(project_id.to_string());
    assert!(project_cache.join(PUBLISH_WORKING_RECOVERY_FILE).is_file());

    let reopened = LiveRevisionStore::open(&canonical, &cache).unwrap();
    assert!(!reopened.initial_publish_pending.get());
    reopened.publish().unwrap();
    assert!(!project_cache.join(PUBLISH_WORKING_RECOVERY_FILE).exists());
}

#[test]
fn recovery_protocol_crash_child() {
    let Ok(canonical) = std::env::var("SPECTRUM_RECOVERY_CRASH_CANONICAL") else {
        return;
    };
    let cache = std::env::var("SPECTRUM_RECOVERY_CRASH_CACHE").unwrap();
    let fault = match std::env::var("SPECTRUM_RECOVERY_CRASH_FAULT")
        .unwrap()
        .as_str()
    {
        "poison-synced" => PublishFault::WorkingPoisonSynced,
        "marker-synced" => PublishFault::WorkingRecoveryMarkerSynced,
        "poison-removed" => PublishFault::WorkingPoisonRemoved,
        other => panic!("unknown recovery crash fault {other}"),
    };
    let mut live = LiveRevisionStore::open(
        std::path::Path::new(&canonical),
        std::path::Path::new(&cache),
    )
    .unwrap();
    PUBLISH_FAULT.set(Some(PublishFault::CandidateSynced));
    RECOVERY_FAULT.set(Some(fault));
    RECOVERY_CRASH_MODE.set(Some(CrashMode::Kill));
    live.mutate(|store| {
        store.put_asset(
            "application/x-recovery-crash",
            b"recovery protocol residual",
        )
    })
    .unwrap();
    panic!("recovery fault {fault:?} did not terminate the child");
}

#[test]
fn real_process_death_leaves_each_recovery_protocol_boundary_fail_closed_or_exact() {
    for (fault, poison_expected, marker_expected, reopen_fails) in [
        ("poison-synced", true, false, true),
        ("marker-synced", true, true, true),
        ("poison-removed", false, true, false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let canonical = directory.path().join("project.lumen");
        let cache = directory.path().join("cache");
        let (live, info) = LiveRevisionStore::create(
            &canonical,
            &cache,
            project(&format!("spectrum.recovery-crash-{fault}")),
        )
        .unwrap();
        let base = RevisionStore::inspect(&canonical).unwrap();
        let project_cache = cache.join(info.project_id.to_string());
        drop(live);

        let status = Command::new(std::env::current_exe().unwrap())
            .arg("recovery_protocol_crash_child")
            .arg("--nocapture")
            .env("SPECTRUM_RECOVERY_CRASH_CANONICAL", &canonical)
            .env("SPECTRUM_RECOVERY_CRASH_CACHE", &cache)
            .env("SPECTRUM_RECOVERY_CRASH_FAULT", fault)
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL), "{fault}");
        assert_eq!(
            project_cache.join(PUBLISH_WORKING_POISON_FILE).exists(),
            poison_expected,
            "{fault}"
        );
        assert_eq!(
            project_cache.join(PUBLISH_WORKING_RECOVERY_FILE).exists(),
            marker_expected,
            "{fault}"
        );
        assert_eq!(
            RevisionStore::inspect(&canonical).unwrap().generation,
            base.generation,
            "{fault}"
        );
        assert!(
            RevisionStore::inspect(&project_cache.join(STORE_FILE))
                .unwrap()
                .generation
                > base.generation,
            "{fault}"
        );

        let reopened = LiveRevisionStore::open(&canonical, &cache);
        assert_eq!(reopened.is_err(), reopen_fails, "{fault}");
        if let Ok(reopened) = reopened {
            let recovered = Asset::new(
                "application/x-recovery-crash",
                b"recovery protocol residual".to_vec(),
            );
            assert!(reopened.store().asset(recovered.id).unwrap().is_some());
            assert_eq!(
                RevisionStore::inspect(&canonical).unwrap().generation,
                reopened.store().generation().unwrap()
            );
            assert!(!project_cache.join(PUBLISH_WORKING_RECOVERY_FILE).exists());
        }
    }
}

#[test]
fn open_preserves_higher_working_state_when_any_publication_marker_is_bad() {
    for (marker_name, marker_bytes) in [
        (PUBLISH_CURRENT_FILE, b"1:-".as_slice()),
        (
            PUBLISH_WORKING_RECOVERY_FILE,
            b"not-a-valid-recovery-marker".as_slice(),
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let canonical = directory.path().join("project.lumen");
        let stale = directory.path().join("stale.lumen");
        let cache = directory.path().join("cache");
        let (mut live, info) = LiveRevisionStore::create(
            &canonical,
            &cache,
            project(&format!("spectrum.bad-marker-{marker_name}")),
        )
        .unwrap();
        std::fs::copy(&canonical, &stale).unwrap();
        live.mutate(|store| store.put_asset("application/x-bad-marker", marker_name.as_bytes()))
            .unwrap();
        let working_path = live.working_path().to_owned();
        let project_cache = cache.join(info.project_id.to_string());
        drop(live);

        std::fs::write(project_cache.join(marker_name), marker_bytes).unwrap();
        std::fs::copy(&stale, &canonical).unwrap();
        let before_bytes = std::fs::read(&working_path).unwrap();
        let before = RevisionStore::inspect(&working_path).unwrap();

        assert!(LiveRevisionStore::open(&canonical, &cache).is_err());
        assert_eq!(std::fs::read(&working_path).unwrap(), before_bytes);
        let after = RevisionStore::inspect(&working_path).unwrap();
        assert_eq!(after.generation, before.generation);
        assert_eq!(after.state_id, before.state_id);
    }
}

#[test]
fn working_marker_cleanup_crash_child() {
    let Ok(canonical) = std::env::var("SPECTRUM_CLEANUP_CRASH_CANONICAL") else {
        return;
    };
    let cache = std::env::var("SPECTRUM_CLEANUP_CRASH_CACHE").unwrap();
    let mut live = LiveRevisionStore::open(
        std::path::Path::new(&canonical),
        std::path::Path::new(&cache),
    )
    .unwrap();
    PUBLISH_FAULT.set(Some(PublishFault::CandidateSynced));
    live.mutate(|store| {
        store.put_asset(
            "application/x-cleanup-crash",
            b"published before abrupt death",
        )
    })
    .unwrap();
    PUBLISH_FAULT.set(None);
    live.publish().unwrap();
    unsafe {
        libc::raise(libc::SIGKILL);
        libc::_exit(87);
    }
}

#[test]
fn published_recovery_durably_removes_the_working_marker_before_process_death() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let (live, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        project("spectrum.working-marker-cleanup"),
    )
    .unwrap();
    let project_cache = cache.join(info.project_id.to_string());
    drop(live);

    let status = Command::new(std::env::current_exe().unwrap())
        .arg("working_marker_cleanup_crash_child")
        .arg("--nocapture")
        .env("SPECTRUM_CLEANUP_CRASH_CANONICAL", &canonical)
        .env("SPECTRUM_CLEANUP_CRASH_CACHE", &cache)
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert!(!project_cache.join(PUBLISH_WORKING_RECOVERY_FILE).exists());

    let recovered = Asset::new(
        "application/x-cleanup-crash",
        b"published before abrupt death".to_vec(),
    );
    let reopened = LiveRevisionStore::open(&canonical, &cache).unwrap();
    assert!(reopened.store().asset(recovered.id).unwrap().is_some());
    assert!(!project_cache.join(PUBLISH_WORKING_RECOVERY_FILE).exists());
}

#[test]
fn failed_publication_marks_exact_working_state_for_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let (mut live, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        project("spectrum.failed-publication-recovery"),
    )
    .unwrap();
    let base_generation = RevisionStore::inspect(&canonical).unwrap().generation;
    let project_cache = cache.join(info.project_id.to_string());
    let recovered_asset = Asset::new(
        "application/x-failed-publication",
        b"durable working state".to_vec(),
    );

    PUBLISH_FAULT.set(Some(PublishFault::CandidateSynced));
    live.mutate(|store| {
        store.put_asset("application/x-failed-publication", b"durable working state")
    })
    .unwrap();
    PUBLISH_FAULT.set(None);

    assert!(live.pending_publish_error().is_some());
    assert_eq!(
        RevisionStore::inspect(&canonical).unwrap().generation,
        base_generation
    );
    let working = RevisionStore::inspect(live.working_path()).unwrap();
    assert!(working.generation > base_generation);
    assert!(working_recovery_marker_matches(
        &PrivateDirectory::open(&project_cache).unwrap(),
        working.generation,
        working.state_id,
    ));
    drop(live);

    let recovered = LiveRevisionStore::open(&canonical, &cache).unwrap();
    assert!(
        recovered
            .store()
            .asset(recovered_asset.id)
            .unwrap()
            .is_some()
    );
    let canonical_after_recovery = RevisionStore::inspect(&canonical).unwrap();
    assert_eq!(canonical_after_recovery.generation, working.generation);
    assert_eq!(canonical_after_recovery.state_id, working.state_id);
    assert!(!project_cache.join(PUBLISH_WORKING_RECOVERY_FILE).exists());
}

#[test]
fn abrupt_pre_exchange_death_discards_unmarked_higher_working_cache() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let (live, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        project("spectrum.unmarked-working-recovery"),
    )
    .unwrap();
    let base = RevisionStore::inspect(&canonical).unwrap();
    let project_cache = cache.join(info.project_id.to_string());
    drop(live);

    let status = Command::new(std::env::current_exe().unwrap())
        .arg("publication_crash_child")
        .arg("--nocapture")
        .env("SPECTRUM_CRASH_CANONICAL", &canonical)
        .env("SPECTRUM_CRASH_CACHE", &cache)
        .env("SPECTRUM_CRASH_FAULT", "candidate-synced")
        .env("SPECTRUM_CRASH_MODE", "kill")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert_eq!(
        RevisionStore::inspect(&canonical).unwrap().generation,
        base.generation
    );
    assert!(!project_cache.join(PUBLISH_WORKING_RECOVERY_FILE).exists());

    let discarded_asset = Asset::new(
        "application/x-crash-child",
        b"survives abrupt death".to_vec(),
    );
    let recovered = LiveRevisionStore::open(&canonical, &cache).unwrap();
    assert!(
        recovered
            .store()
            .asset(discarded_asset.id)
            .unwrap()
            .is_none()
    );
    let recovered_working = RevisionStore::inspect(recovered.working_path()).unwrap();
    assert_eq!(recovered_working.generation, base.generation);
    assert_eq!(recovered_working.state_id, base.state_id);
}

#[test]
fn pending_recovery_is_published_before_the_next_mutation_can_overwrite_it() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let (live, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        project("spectrum.acknowledged-prefix-recovery"),
    )
    .unwrap();
    let base_generation = live.store().generation().unwrap();
    let project_cache = cache.join(info.project_id.to_string());
    let tail_started = directory.path().join("tail-mutation-started");
    drop(live);

    let status = Command::new(std::env::current_exe().unwrap())
        .arg("publication_crash_child")
        .arg("--nocapture")
        .env("SPECTRUM_CRASH_CANONICAL", &canonical)
        .env("SPECTRUM_CRASH_CACHE", &cache)
        .env("SPECTRUM_CRASH_PRIME_HANDLED_FAILURE", "1")
        .env("SPECTRUM_CRASH_TAIL_SENTINEL", &tail_started)
        .env("SPECTRUM_CRASH_FAULT", "candidate-synced")
        .env("SPECTRUM_CRASH_MODE", "kill")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert!(
        tail_started.exists(),
        "the G+1 mutation closure did not run"
    );
    assert_eq!(
        RevisionStore::inspect(&canonical).unwrap().generation,
        base_generation + 1
    );
    assert!(!project_cache.join(PUBLISH_WORKING_RECOVERY_FILE).exists());

    let durable_prefix = Asset::new(
        "application/x-durable-prefix",
        b"acknowledged before the next mutation".to_vec(),
    );
    let unacknowledged_tail = Asset::new(
        "application/x-crash-child",
        b"survives abrupt death".to_vec(),
    );
    let recovered = LiveRevisionStore::open(&canonical, &cache).unwrap();
    assert!(
        recovered
            .store()
            .asset(durable_prefix.id)
            .unwrap()
            .is_some()
    );
    assert!(
        recovered
            .store()
            .asset(unacknowledged_tail.id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn shared_lock_publishes_another_store_recovery_before_entering_the_next_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let (mut first, _) = LiveRevisionStore::create(
        &canonical,
        &cache,
        project("spectrum.cross-store-prefix-recovery"),
    )
    .unwrap();
    let base_generation = first.store().generation().unwrap();
    let second_opened = directory.path().join("second-opened");
    let allow_second = directory.path().join("allow-second");
    let second_mutated = directory.path().join("second-mutated");
    let mut second = Command::new(std::env::current_exe().unwrap())
        .arg("publication_crash_child")
        .arg("--nocapture")
        .env("SPECTRUM_CRASH_CANONICAL", &canonical)
        .env("SPECTRUM_CRASH_CACHE", &cache)
        .env("SPECTRUM_CRASH_OPEN_SENTINEL", &second_opened)
        .env("SPECTRUM_CRASH_WAIT_FOR", &allow_second)
        .env("SPECTRUM_CRASH_ARM_AFTER_MUTATION", "1")
        .env("SPECTRUM_CRASH_TAIL_SENTINEL", &second_mutated)
        .env("SPECTRUM_CRASH_FAULT", "candidate-synced")
        .env("SPECTRUM_CRASH_MODE", "kill")
        .spawn()
        .unwrap();
    for _ in 0..1_000 {
        if second_opened.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(second_opened.exists(), "second store did not open");

    PUBLISH_FAULT.set(Some(PublishFault::CandidateSynced));
    first
        .mutate(|store| {
            store.put_asset(
                "application/x-cross-store-prefix",
                b"acknowledged by first store",
            )
        })
        .unwrap();
    PUBLISH_FAULT.set(None);
    assert!(first.pending_publish_error().is_some());
    fs::write(&allow_second, b"continue").unwrap();

    let status = second.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert!(
        second_mutated.exists(),
        "second mutation closure never ran after recovery publication"
    );
    assert_eq!(
        RevisionStore::inspect(&canonical).unwrap().generation,
        base_generation + 1
    );
    drop(first);

    let prefix = Asset::new(
        "application/x-cross-store-prefix",
        b"acknowledged by first store".to_vec(),
    );
    let discarded_tail = Asset::new(
        "application/x-crash-child",
        b"survives abrupt death".to_vec(),
    );
    let recovered = LiveRevisionStore::open(&canonical, &cache).unwrap();
    assert!(recovered.store().asset(prefix.id).unwrap().is_some());
    assert!(
        recovered
            .store()
            .asset(discarded_tail.id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn mutate_reports_failure_when_working_recovery_marker_cannot_be_written() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let (mut live, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        project("spectrum.failed-recovery-marker"),
    )
    .unwrap();
    let base_generation = live.store().generation().unwrap();
    let project_cache = cache.join(info.project_id.to_string());
    PUBLISH_FAULT.set(Some(PublishFault::CandidateSynced));
    RECOVERY_FAULT.set(Some(PublishFault::WorkingRecoveryMarkerInstall));
    let mut closure_ran = false;
    let error = live
        .mutate(|store| {
            closure_ran = true;
            store.put_asset("application/x-unacknowledged", b"not acknowledged")
        })
        .unwrap_err();
    PUBLISH_FAULT.set(None);
    RECOVERY_FAULT.set(None);

    assert!(closure_ran, "the marker fault must happen after preflight");
    assert!(
        error
            .to_string()
            .contains("could not preserve failed publication")
    );
    assert!(
        live.pending_publish_error()
            .unwrap()
            .contains("could not preserve failed publication")
    );
    assert!(project_cache.join(PUBLISH_WORKING_POISON_FILE).exists());
    assert!(!project_cache.join(PUBLISH_WORKING_RECOVERY_FILE).exists());

    assert!(live.publish().is_err());
    let mut retry_ran = false;
    assert!(
        live.mutate(|_| {
            retry_ran = true;
            Ok(())
        })
        .is_err()
    );
    assert!(!retry_ran, "a poisoned store entered a retry closure");
    drop(live);
    assert!(LiveRevisionStore::open(&canonical, &cache).is_err());

    let retry_cache = directory.path().join("retry-cache");
    let mut retry = LiveRevisionStore::open(&canonical, &retry_cache).unwrap();
    retry
        .mutate(|store| store.put_asset("application/x-unacknowledged", b"retried exactly once"))
        .unwrap();
    assert_eq!(
        RevisionStore::inspect(&canonical).unwrap().generation,
        base_generation + 1
    );
}

#[test]
fn recovery_protocol_faults_never_turn_an_ordinary_error_into_a_later_publication() {
    let poison_faults = [
        PublishFault::WorkingPoisonRenamed,
        PublishFault::WorkingPoisonSynced,
        PublishFault::WorkingRecoveryMarkerRenamed,
        PublishFault::WorkingRecoveryMarkerSynced,
    ];
    for fault in poison_faults {
        let directory = tempfile::tempdir().unwrap();
        let canonical = directory.path().join("project.lumen");
        let cache = directory.path().join("cache");
        let (mut live, info) = LiveRevisionStore::create(
            &canonical,
            &cache,
            project(&format!("spectrum.recovery-boundary-{fault:?}")),
        )
        .unwrap();
        let base_generation = live.store().generation().unwrap();
        let project_cache = cache.join(info.project_id.to_string());

        PUBLISH_FAULT.set(Some(PublishFault::CandidateSynced));
        RECOVERY_FAULT.set(Some(fault));
        let error = live
            .mutate(|store| store.put_asset("application/x-boundary", b"must not publish"))
            .unwrap_err();
        PUBLISH_FAULT.set(None);
        RECOVERY_FAULT.set(None);

        assert!(
            error
                .to_string()
                .contains("could not preserve failed publication")
        );
        assert!(project_cache.join(PUBLISH_WORKING_POISON_FILE).exists());
        assert!(live.publish().is_err());
        drop(live);
        assert!(LiveRevisionStore::open(&canonical, &cache).is_err());
        assert_eq!(
            RevisionStore::inspect(&canonical).unwrap().generation,
            base_generation
        );
    }
}

#[test]
fn poison_cleanup_ambiguity_is_acknowledged_as_pending_not_returned_as_retryable_error() {
    for fault in [
        PublishFault::WorkingPoisonRemoved,
        PublishFault::WorkingPoisonRemovalSynced,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let canonical = directory.path().join("project.lumen");
        let cache = directory.path().join("cache");
        let (mut live, _) = LiveRevisionStore::create(
            &canonical,
            &cache,
            project(&format!("spectrum.cleanup-boundary-{fault:?}")),
        )
        .unwrap();

        PUBLISH_FAULT.set(Some(PublishFault::CandidateSynced));
        RECOVERY_FAULT.set(Some(fault));
        live.mutate(|store| store.put_asset("application/x-boundary", b"acknowledged pending"))
            .unwrap();
        PUBLISH_FAULT.set(None);
        RECOVERY_FAULT.set(None);

        assert!(live.pending_publish_error().is_some());
        live.publish().unwrap();
        assert!(live.pending_publish_error().is_none());
    }
}
