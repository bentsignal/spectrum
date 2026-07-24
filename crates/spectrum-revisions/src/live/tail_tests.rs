use std::{fs, os::unix::process::ExitStatusExt, process::Command};

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
        assert!(project_cache.join(PUBLISH_MIRROR_READY_FILE).exists());

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
