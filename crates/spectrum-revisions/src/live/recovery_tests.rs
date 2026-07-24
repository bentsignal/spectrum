use std::{os::unix::process::ExitStatusExt as _, process::Command};

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
    let project_cache = cache.join(info.project_id.to_string());
    let blocked_marker = project_cache.join(PUBLISH_WORKING_RECOVERY_FILE);
    std::fs::create_dir(&blocked_marker).unwrap();

    PUBLISH_FAULT.set(Some(PublishFault::CandidateSynced));
    let error = live
        .mutate(|store| store.put_asset("application/x-unacknowledged", b"not acknowledged"))
        .unwrap_err();
    PUBLISH_FAULT.set(None);

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

    std::fs::remove_dir(&blocked_marker).unwrap();
    live.publish().unwrap();
    assert!(live.pending_publish_error().is_none());
}
