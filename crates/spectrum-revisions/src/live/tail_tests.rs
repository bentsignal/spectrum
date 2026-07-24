use std::{
    fs,
    os::unix::{
        fs::{MetadataExt as _, PermissionsExt as _},
        process::ExitStatusExt,
    },
    process::Command,
};

use super::*;
use crate::{Actor, ActorKind, Asset, Payload};

fn project(application_id: &str) -> NewProject {
    NewProject {
        application_id: application_id.into(),
        application_version: "1.0.0".into(),
        actor: Actor {
            id: format!("{application_id}:actor"),
            display_name: "Publication tail test".into(),
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
        assets: vec![Asset::new(
            "application/x-baseline",
            vec![0x44; 1024 * 1024],
        )],
    }
}

#[test]
fn exchange_capability_is_probed_once_per_live_store() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let (mut live, _) =
        LiveRevisionStore::create(&canonical, &cache, project("spectrum.probe-cache")).unwrap();

    live.mutate(|store| store.put_asset("application/x-first", b"first"))
        .unwrap();
    assert_eq!(live.publish_capabilities.exchange_probe_count(), 1);
    live.mutate(|store| store.put_asset("application/x-second", b"second"))
        .unwrap();
    assert_eq!(live.publish_capabilities.exchange_probe_count(), 1);
    drop(live);

    let mut reopened = LiveRevisionStore::open(&canonical, &cache).unwrap();
    reopened
        .mutate(|store| store.put_asset("application/x-third", b"third"))
        .unwrap();
    assert_eq!(reopened.publish_capabilities.exchange_probe_count(), 1);
}

#[test]
fn abandoned_exchange_probe_entries_are_ignored_by_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let cache = directory.path().join("cache");
    let (live, info) =
        LiveRevisionStore::create(&canonical, &cache, project("spectrum.probe-residual")).unwrap();
    let project_cache = cache.join(info.project_id.to_string());
    let first_residual = project_cache.join(".exchange-probe-a-abandoned");
    let second_residual = project_cache.join(".exchange-probe-b-abandoned");
    fs::write(&first_residual, b"not publication state").unwrap();
    fs::write(&second_residual, b"not publication state").unwrap();
    drop(live);

    let mut reopened = LiveRevisionStore::open(&canonical, &cache).unwrap();
    reopened
        .mutate(|store| store.put_asset("application/x-after-residual", b"committed"))
        .unwrap();

    assert!(reopened.pending_publish_error().is_none());
    assert_eq!(fs::read(first_residual).unwrap(), b"not publication state");
    assert_eq!(fs::read(second_residual).unwrap(), b"not publication state");
    assert!(
        RevisionStore::open_read_only(&canonical)
            .unwrap()
            .asset(Asset::new("application/x-after-residual", b"committed".to_vec()).id)
            .unwrap()
            .is_some()
    );
}

#[test]
fn copy_over_stale_canonical_xattr_is_non_authorizing() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let stale = directory.path().join("stale.lumen");
    let cache = directory.path().join("cache");
    let (mut live, info) =
        LiveRevisionStore::create(&canonical, &cache, project("spectrum.copy-over-proof")).unwrap();
    let base = RevisionStore::inspect(&canonical).unwrap();
    fs::copy(&canonical, &stale).unwrap();
    let added = Asset::new("application/x-copy-over", b"newer".to_vec());

    live.mutate(|store| store.put_asset(&added.media_type, &added.bytes))
        .unwrap();
    let published = RevisionStore::inspect(&canonical).unwrap();
    assert!(exchange_proof_matches(&canonical, published.generation, published.state_id).unwrap());
    let project_cache = cache.join(info.project_id.to_string());
    remove_private_file(
        &PrivateDirectory::open(&project_cache).unwrap(),
        PUBLISH_CURRENT_FILE,
    )
    .unwrap();
    let canonical_inode = canonical.metadata().unwrap().ino();
    fs::copy(&stale, &canonical).unwrap();
    assert_eq!(canonical.metadata().unwrap().ino(), canonical_inode);
    assert!(!exchange_proof_matches(&canonical, base.generation, base.state_id).unwrap());
    recover_exchange(
        &PrivateDirectory::open(&project_cache).unwrap(),
        &canonical,
        &project_cache,
    )
    .unwrap();
    drop(live);

    let reopened = LiveRevisionStore::open(&canonical, &cache).unwrap();
    assert_eq!(reopened.store().generation().unwrap(), base.generation);
    assert!(reopened.store().asset(added.id).unwrap().is_none());
}

#[test]
fn equal_generation_returned_copy_wins_over_stale_local_proof() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let traveler = directory.path().join("traveler.lumen");
    let cache = directory.path().join("cache");
    let remote_cache = directory.path().join("remote-cache");
    let (mut local, _) =
        LiveRevisionStore::create(&canonical, &cache, project("spectrum.returned-copy")).unwrap();
    fs::copy(&canonical, &traveler).unwrap();
    let mut remote = LiveRevisionStore::open(&traveler, &remote_cache).unwrap();
    let local_asset = Asset::new("application/x-local", b"local".to_vec());
    let remote_asset = Asset::new("application/x-remote", b"remote".to_vec());

    local
        .mutate(|store| store.put_asset(&local_asset.media_type, &local_asset.bytes))
        .unwrap();
    remote
        .mutate(|store| store.put_asset(&remote_asset.media_type, &remote_asset.bytes))
        .unwrap();
    let local_state = RevisionStore::inspect(&canonical).unwrap();
    let returned_state = RevisionStore::inspect(&traveler).unwrap();
    assert_eq!(local_state.generation, returned_state.generation);
    assert_ne!(local_state.state_id, returned_state.state_id);
    drop(local);
    drop(remote);

    let canonical_inode = canonical.metadata().unwrap().ino();
    fs::copy(&traveler, &canonical).unwrap();
    assert_eq!(canonical.metadata().unwrap().ino(), canonical_inode);
    assert!(
        !exchange_proof_matches(
            &canonical,
            returned_state.generation,
            returned_state.state_id,
        )
        .unwrap()
    );
    let reopened = LiveRevisionStore::open(&canonical, &cache).unwrap();
    assert_eq!(
        reopened.store().state_id().unwrap(),
        returned_state.state_id
    );
    assert!(reopened.store().asset(local_asset.id).unwrap().is_none());
    assert!(reopened.store().asset(remote_asset.id).unwrap().is_some());
}

#[test]
fn corrupted_private_predecessor_is_reconstructed_before_exchange() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let stale = directory.path().join("stale.lumen");
    let cache = directory.path().join("cache");
    let (mut live, info) =
        LiveRevisionStore::create(&canonical, &cache, project("spectrum.corrupt-predecessor"))
            .unwrap();
    fs::copy(&canonical, &stale).unwrap();
    live.mutate(|store| store.put_asset("application/x-first", b"first"))
        .unwrap();
    let project_cache = cache.join(info.project_id.to_string());
    let mirror = project_cache.join(PUBLISH_MIRROR_FILE);
    let mirror_len = mirror.metadata().unwrap().len();
    fs::write(&mirror, vec![0x3c; mirror_len as usize]).unwrap();

    live.mutate(|store| store.put_asset("application/x-second", b"second"))
        .unwrap();

    assert!(live.pending_publish_error().is_none());
    assert!(live.last_publish_stats().incremental);
    assert_eq!(
        fs::read(&canonical).unwrap(),
        fs::read(live.working_path()).unwrap()
    );

    fs::copy(&stale, &mirror).unwrap();
    fs::set_permissions(&canonical, fs::Permissions::from_mode(0o400)).unwrap();
    live.mutate(|store| store.put_asset("application/x-third", b"third"))
        .unwrap();

    assert!(live.pending_publish_error().is_none());
    assert!(live.last_publish_stats().incremental);
    assert_eq!(canonical.metadata().unwrap().mode() & 0o777, 0o400);
    let published = RevisionStore::inspect(&canonical).unwrap();
    assert_eq!(published.generation, live.store().generation().unwrap());
    assert_eq!(published.state_id, live.store().state_id().unwrap());
    assert_eq!(
        fs::read(&canonical).unwrap(),
        fs::read(live.working_path()).unwrap()
    );
}

#[test]
fn valid_future_private_slot_is_rejected_without_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("project.lumen");
    let future = directory.path().join("future.lumen");
    let cache = directory.path().join("cache");
    let future_cache = directory.path().join("future-cache");
    let (mut live, info) =
        LiveRevisionStore::create(&canonical, &cache, project("spectrum.future-slot")).unwrap();
    live.mutate(|store| store.put_asset("application/x-first", b"first"))
        .unwrap();
    let published = RevisionStore::inspect(&canonical).unwrap();
    assert!(exchange_proof_matches(&canonical, published.generation, published.state_id).unwrap());
    fs::copy(&canonical, &future).unwrap();

    let mut future_live = LiveRevisionStore::open(&future, &future_cache).unwrap();
    future_live
        .mutate(|store| store.put_asset("application/x-second", b"second"))
        .unwrap();
    future_live
        .mutate(|store| store.put_asset("application/x-third", b"third"))
        .unwrap();
    drop(future_live);
    let future_inspection = RevisionStore::inspect(&future).unwrap();
    assert!(future_inspection.generation > published.generation);

    let project_cache = cache.join(info.project_id.to_string());
    let mirror = project_cache.join(PUBLISH_MIRROR_FILE);
    let mirror_inode = mirror.metadata().unwrap().ino();
    fs::copy(&future, &mirror).unwrap();
    assert_eq!(mirror.metadata().unwrap().ino(), mirror_inode);
    let mirror_bytes = fs::read(&mirror).unwrap();
    let canonical_bytes = fs::read(&canonical).unwrap();
    drop(live);

    let error = recover_exchange(
        &PrivateDirectory::open(&project_cache).unwrap(),
        &canonical,
        &project_cache,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RevisionError::Invalid(message)
            if message
                == "committed inode-bound publication slot is newer than its canonical target"
    ));
    assert_eq!(mirror.metadata().unwrap().ino(), mirror_inode);
    assert_eq!(fs::read(&mirror).unwrap(), mirror_bytes);
    assert_eq!(fs::read(&canonical).unwrap(), canonical_bytes);
}

#[test]
fn intent_removed_abrupt_death_recovers_exact_committed_state() {
    for mode in ["exit", "abort", "kill"] {
        let directory = tempfile::tempdir().unwrap();
        let canonical = directory.path().join("project.lumen");
        let cache = directory.path().join("cache");
        let (live, info) = LiveRevisionStore::create(
            &canonical,
            &cache,
            project(&format!("spectrum.intent-removed-{mode}")),
        )
        .unwrap();
        let base_generation = live.store().generation().unwrap();
        let project_cache = cache.join(info.project_id.to_string());
        drop(live);

        let status = Command::new(std::env::current_exe().unwrap())
            .arg("publication_crash_child")
            .arg("--nocapture")
            .env("SPECTRUM_CRASH_CANONICAL", &canonical)
            .env("SPECTRUM_CRASH_CACHE", &cache)
            .env("SPECTRUM_CRASH_FAULT", "intent-removed")
            .env("SPECTRUM_CRASH_MODE", mode)
            .status()
            .unwrap();
        match mode {
            "exit" => assert_eq!(status.code(), Some(86)),
            "abort" => assert_eq!(status.signal(), Some(libc::SIGABRT)),
            "kill" => assert_eq!(status.signal(), Some(libc::SIGKILL)),
            _ => unreachable!(),
        }

        assert!(RevisionStore::inspect(&canonical).unwrap().generation > base_generation);
        let published = RevisionStore::inspect(&canonical).unwrap();
        assert!(
            exchange_proof_matches(&canonical, published.generation, published.state_id).unwrap()
        );

        let child_asset = Asset::new(
            "application/x-crash-child",
            b"survives abrupt death".to_vec(),
        );
        let mut recovered = LiveRevisionStore::open(&canonical, &cache).unwrap();
        assert!(!project_cache.join(PUBLISH_EXCHANGE_INTENT_FILE).exists());
        assert!(recovered.store().asset(child_asset.id).unwrap().is_some());
        recovered
            .mutate(|store| store.put_asset("application/x-followup", mode.as_bytes()))
            .unwrap();
        assert!(recovered.pending_publish_error().is_none());
        assert!(!project_cache.join(PUBLISH_EXCHANGE_INTENT_FILE).exists());
        assert_eq!(
            RevisionStore::inspect(&canonical).unwrap().generation,
            RevisionStore::inspect(recovered.working_path())
                .unwrap()
                .generation
        );
        assert!(fs::metadata(&canonical).unwrap().is_file());
    }
}
