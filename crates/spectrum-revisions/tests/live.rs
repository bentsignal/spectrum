use std::{ffi::OsString, fs, path::Path};

use spectrum_revisions::{
    Actor, ActorKind, AppendRevision, Encoding, LiveRevisionStore, NewProject, Payload, RevisionId,
    SessionId, TrackId,
};
#[cfg(target_os = "linux")]
use spectrum_revisions::{Asset, PublishStats, PublishStrategy, RevisionStore};

struct Fixture {
    directory: tempfile::TempDir,
    canonical: std::path::PathBuf,
    cache: std::path::PathBuf,
    live: LiveRevisionStore,
    root: RevisionId,
    track: TrackId,
    human_session: SessionId,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let canonical = directory.path().join("project.prism");
        let cache = directory.path().join("private-cache");
        let human_session = SessionId::new();
        let (live, info) = LiveRevisionStore::create(
            &canonical,
            &cache,
            NewProject {
                application_id: "spectrum.test".into(),
                application_version: "1.0.0".into(),
                actor: actor("person:1", ActorKind::Human),
                session_id: human_session,
                root_label: Some("Created".into()),
                track_kind: "test.document".into(),
                track_label: "Document".into(),
                initial_snapshots: vec![payload("test.snapshot", b"root")],
                assets: Vec::new(),
            },
        )
        .unwrap();
        Self {
            directory,
            canonical,
            cache,
            live,
            root: info.root_revision,
            track: info.default_track_id,
            human_session,
        }
    }

    fn append(&mut self, session_id: SessionId, parent: RevisionId, label: &str) -> RevisionId {
        self.live
            .mutate(|store| {
                store.append(AppendRevision {
                    track_id: self.track,
                    session_id,
                    expected_parent: parent,
                    application_version: "1.0.0".into(),
                    label: Some(label.into()),
                    command_count: 1,
                    operation_payloads: vec![payload("test.operations", label.as_bytes())],
                    snapshots: Vec::new(),
                    assets: Vec::new(),
                })
            })
            .unwrap()
            .id
    }
}

#[test]
fn live_store_keeps_transaction_sidecars_out_of_the_project_folder() {
    let fixture = Fixture::new();
    drop(fixture.live);
    let stale_publish = fixture
        .directory
        .path()
        .join(".project.prism.spectrum-publish-stale.tmp");
    fs::write(&stale_publish, b"interrupted publish").unwrap();
    let mut live = LiveRevisionStore::open(&fixture.canonical, &fixture.cache).unwrap();
    live.mutate(|store| {
        store.append(AppendRevision {
            track_id: fixture.track,
            session_id: fixture.human_session,
            expected_parent: fixture.root,
            application_version: "1.0.0".into(),
            label: Some("Edit".into()),
            command_count: 1,
            operation_payloads: vec![payload("test.operations", b"edit")],
            snapshots: Vec::new(),
            assets: Vec::new(),
        })
    })
    .unwrap();

    assert!(fixture.canonical.is_file());
    assert!(!sidecar(&fixture.canonical, "-wal").exists());
    assert!(!sidecar(&fixture.canonical, "-shm").exists());
    assert!(!stale_publish.exists());
    assert!(sidecar(live.working_path(), "-wal").exists());
    assert!(sidecar(live.working_path(), "-shm").exists());
    assert_eq!(
        fs::read(&fixture.canonical).unwrap(),
        fs::read(live.working_path()).unwrap()
    );

    let portable = fixture.directory.path().join("portable-copy.prism");
    fs::copy(&fixture.canonical, &portable).unwrap();
    let reopened = LiveRevisionStore::open(
        &portable,
        &fixture.directory.path().join("second-private-cache"),
    )
    .unwrap();
    assert_eq!(reopened.store().children(fixture.root).unwrap().len(), 1);
    assert!(!sidecar(&portable, "-wal").exists());
    assert!(!sidecar(&portable, "-shm").exists());
}

#[test]
fn independent_live_connections_publish_both_branches_without_visible_sidecars() {
    let mut fixture = Fixture::new();
    let mut agent = LiveRevisionStore::open(&fixture.canonical, &fixture.cache).unwrap();
    let agent_session = SessionId::new();
    agent
        .mutate(|store| {
            store.resume_session(
                agent_session,
                actor("agent:1", ActorKind::Agent),
                fixture.root,
            )
        })
        .unwrap();

    let human = fixture.append(fixture.human_session, fixture.root, "Human edit");
    let agent_revision = agent
        .mutate(|store| {
            store.append(AppendRevision {
                track_id: fixture.track,
                session_id: agent_session,
                expected_parent: fixture.root,
                application_version: "1.0.0".into(),
                label: Some("Agent edit".into()),
                command_count: 1,
                operation_payloads: vec![payload("test.operations", b"agent")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
        })
        .unwrap()
        .id;

    let children = fixture.live.store().children(fixture.root).unwrap();
    assert_eq!(children.len(), 2);
    assert!(children.iter().any(|revision| revision.id == human));
    assert!(
        children
            .iter()
            .any(|revision| revision.id == agent_revision)
    );
    assert!(!sidecar(&fixture.canonical, "-wal").exists());
    assert!(!sidecar(&fixture.canonical, "-shm").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn same_cache_publishers_rebase_and_explicit_retry_converges() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("shared.prism");
    let cache = directory.path().join("shared-cache");
    let (mut first, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        NewProject {
            application_id: "spectrum.test".into(),
            application_version: "1.0.0".into(),
            actor: actor("person:1", ActorKind::Human),
            session_id: SessionId::new(),
            root_label: Some("Created".into()),
            track_kind: "test.document".into(),
            track_label: "Document".into(),
            initial_snapshots: vec![payload("test.snapshot", b"root")],
            assets: Vec::new(),
        },
    )
    .unwrap();
    let mut second = LiveRevisionStore::open(&canonical, &cache).unwrap();
    let first_asset = Asset::new("application/x-first-client", b"first".to_vec());
    let second_asset = Asset::new("application/x-second-client", b"second".to_vec());

    first
        .mutate(|store| store.put_asset(&first_asset.media_type, &first_asset.bytes))
        .unwrap();
    second
        .mutate(|store| store.put_asset(&second_asset.media_type, &second_asset.bytes))
        .unwrap();
    assert!(second.pending_publish_error().is_none());
    assert_eq!(
        RevisionStore::open_read_only(&canonical)
            .unwrap()
            .generation()
            .unwrap(),
        second.store().generation().unwrap()
    );

    second.publish().unwrap();
    assert!(second.pending_publish_error().is_none());
    assert_eq!(second.last_publish_stats(), PublishStats::default());
    let project_cache = cache.join(info.project_id.to_string());
    assert!(!project_cache.join("published-exchange.intent").exists());

    drop(first);
    drop(second);
    let verified =
        LiveRevisionStore::open(&canonical, &directory.path().join("verified-cache")).unwrap();
    assert!(verified.store().asset(first_asset.id).unwrap().is_some());
    assert!(verified.store().asset(second_asset.id).unwrap().is_some());
}

#[test]
fn newer_live_cache_recovers_a_stale_published_copy() {
    let mut fixture = Fixture::new();
    let stale = fixture.directory.path().join("stale.prism");
    fs::copy(&fixture.canonical, &stale).unwrap();
    let latest = fixture.append(fixture.human_session, fixture.root, "Survives");
    drop(fixture.live);

    fs::copy(&stale, &fixture.canonical).unwrap();
    let recovered = LiveRevisionStore::open(&fixture.canonical, &fixture.cache).unwrap();
    assert!(recovered.store().revision(latest).unwrap().is_some());
    drop(recovered);

    let separate_cache = fixture.directory.path().join("verified-cache");
    let verified = LiveRevisionStore::open(&fixture.canonical, &separate_cache).unwrap();
    assert!(verified.store().revision(latest).unwrap().is_some());
}

#[test]
fn equally_advanced_returned_copy_replaces_an_inactive_local_cache() {
    let mut fixture = Fixture::new();
    let traveler = fixture.directory.path().join("traveler.prism");
    fs::copy(&fixture.canonical, &traveler).unwrap();
    let traveler_cache = fixture.directory.path().join("traveler-cache");
    let mut remote = LiveRevisionStore::open(&traveler, &traveler_cache).unwrap();

    let local = fixture.append(fixture.human_session, fixture.root, "Local branch");
    let remote_revision = remote
        .mutate(|store| {
            store.append(AppendRevision {
                track_id: fixture.track,
                session_id: fixture.human_session,
                expected_parent: fixture.root,
                application_version: "1.0.0".into(),
                label: Some("Returned branch".into()),
                command_count: 1,
                operation_payloads: vec![payload("test.operations", b"returned")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
        })
        .unwrap()
        .id;
    assert_eq!(
        fixture.live.store().generation().unwrap(),
        remote.store().generation().unwrap()
    );
    drop(remote);
    drop(fixture.live);

    fs::copy(&traveler, &fixture.canonical).unwrap();
    let mut reopened = LiveRevisionStore::open(&fixture.canonical, &fixture.cache).unwrap();
    assert!(
        reopened
            .store()
            .revision(remote_revision)
            .unwrap()
            .is_some()
    );
    assert!(reopened.store().revision(local).unwrap().is_none());
    let after_return = reopened
        .mutate(|store| {
            store.append(AppendRevision {
                track_id: fixture.track,
                session_id: fixture.human_session,
                expected_parent: remote_revision,
                application_version: "1.0.0".into(),
                label: Some("After return".into()),
                command_count: 1,
                operation_payloads: vec![payload("test.operations", b"after-return")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
        })
        .unwrap()
        .id;
    drop(reopened);
    let independently_verified = LiveRevisionStore::open(
        &fixture.canonical,
        &fixture.directory.path().join("returned-verified-cache"),
    )
    .unwrap();
    assert!(
        independently_verified
            .store()
            .revision(after_return)
            .unwrap()
            .is_some()
    );
}

#[cfg(unix)]
#[test]
fn failed_publish_keeps_the_committed_edit_recoverable() {
    use std::os::unix::fs::PermissionsExt;

    let mut fixture = Fixture::new();
    fs::set_permissions(fixture.directory.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let latest = fixture.append(fixture.human_session, fixture.root, "Locally safe");
    assert!(fixture.live.pending_publish_error().is_some());
    assert!(fixture.live.store().revision(latest).unwrap().is_some());

    fs::set_permissions(fixture.directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    fixture.live.publish().unwrap();
    assert!(fixture.live.pending_publish_error().is_none());
    let verified = LiveRevisionStore::open(
        &fixture.canonical,
        &fixture.directory.path().join("verified-after-retry"),
    )
    .unwrap();
    assert!(verified.store().revision(latest).unwrap().is_some());
}

#[cfg(target_os = "linux")]
#[test]
fn small_edits_do_not_rewrite_large_immutable_assets() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("large-project.lumen");
    let cache = directory.path().join("private-cache");
    let session = SessionId::new();
    let original = Asset::new("image/jpeg", vec![0x5a; 8 * 1024 * 1024]);
    let original_id = original.id;
    let (mut live, info) = LiveRevisionStore::create(
        &canonical,
        &cache,
        NewProject {
            application_id: "spectrum.test".into(),
            application_version: "1.0.0".into(),
            actor: actor("person:1", ActorKind::Human),
            session_id: session,
            root_label: Some("Created".into()),
            track_kind: "test.document".into(),
            track_label: "Document".into(),
            initial_snapshots: vec![payload("test.snapshot", b"root")],
            assets: vec![original],
        },
    )
    .unwrap();
    let project_cache = cache.join(info.project_id.to_string());
    let mirror = project_cache.join("published-mirror.sqlite");
    let ready = project_cache.join("published-mirror.ready");
    let backup = project_cache.join("published-backup.sqlite");
    assert_ne!(
        canonical.metadata().unwrap().ino(),
        mirror.metadata().unwrap().ino(),
        "the mutable mirror must never alias the visible project inode"
    );
    assert_eq!(mirror.metadata().unwrap().nlink(), 1);
    fs::write(&backup, b"stale interrupted backup").unwrap();

    let tiny = live
        .mutate(|store| {
            store.append(AppendRevision {
                track_id: info.default_track_id,
                session_id: session,
                expected_parent: info.root_revision,
                application_version: "1.0.0".into(),
                label: Some("Tiny edit".into()),
                command_count: 1,
                operation_payloads: vec![payload("test.operations", b"tiny")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
        })
        .unwrap()
        .id;

    assert!(!backup.exists());
    assert_ne!(
        canonical.metadata().unwrap().ino(),
        mirror.metadata().unwrap().ino(),
        "the previous checkpoint mirror must remain distinct after the atomic rename"
    );
    assert_eq!(mirror.metadata().unwrap().nlink(), 1);
    let stats = live.last_publish_stats();
    assert!(stats.incremental);
    assert!(stats.scanned_bytes > 8 * 1024 * 1024);
    assert!(
        stats.written_bytes < 512 * 1024,
        "tiny edit rewrote {} bytes",
        stats.written_bytes
    );

    let canonical_len = canonical.metadata().unwrap().len();
    fs::write(&mirror, vec![0x3c; canonical_len as usize]).unwrap();
    let after_corruption = live
        .mutate(|store| {
            store.append(AppendRevision {
                track_id: info.default_track_id,
                session_id: session,
                expected_parent: tiny,
                application_version: "1.0.0".into(),
                label: Some("Recover corrupt mirror".into()),
                command_count: 1,
                operation_payloads: vec![payload("test.operations", b"recover-corrupt")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
        })
        .unwrap()
        .id;
    assert!(live.last_publish_stats().incremental);
    assert_eq!(
        fs::read(&canonical).unwrap(),
        fs::read(live.working_path()).unwrap(),
        "page diff must reconstruct even a corrupt cache mirror exactly"
    );

    fs::remove_file(&ready).unwrap();
    fs::write(&mirror, b"interrupted partial mirror").unwrap();
    let after_interruption = live
        .mutate(|store| {
            store.append(AppendRevision {
                track_id: info.default_track_id,
                session_id: session,
                expected_parent: after_corruption,
                application_version: "1.0.0".into(),
                label: Some("Recover interrupted mirror".into()),
                command_count: 1,
                operation_payloads: vec![payload("test.operations", b"recover-interrupted")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
        })
        .unwrap()
        .id;
    assert!(live.last_publish_stats().incremental);
    assert_eq!(
        live.last_publish_stats().strategy,
        spectrum_revisions::PublishStrategy::PageDiffExchange
    );
    assert!(ready.is_file());
    let mut parent = live
        .mutate(|store| {
            store.append(AppendRevision {
                track_id: info.default_track_id,
                session_id: session,
                expected_parent: after_interruption,
                application_version: "1.0.0".into(),
                label: Some("Incremental after recovery".into()),
                command_count: 1,
                operation_payloads: vec![payload("test.operations", b"incremental-again")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
        })
        .unwrap()
        .id;

    assert!(live.last_publish_stats().incremental);
    for mode in [0o444, 0o400] {
        fs::set_permissions(&canonical, fs::Permissions::from_mode(mode)).unwrap();
        parent = live
            .mutate(|store| {
                store.append(AppendRevision {
                    track_id: info.default_track_id,
                    session_id: session,
                    expected_parent: parent,
                    application_version: "1.0.0".into(),
                    label: Some(format!("Preserve mode {mode:o}")),
                    command_count: 1,
                    operation_payloads: vec![payload("test.operations", b"mode")],
                    snapshots: Vec::new(),
                    assets: Vec::new(),
                })
            })
            .unwrap()
            .id;
        assert_eq!(canonical.metadata().unwrap().mode() & 0o777, mode);
        assert!(live.last_publish_stats().incremental);
    }
    assert_eq!(
        live.store().asset(original_id).unwrap().unwrap().len(),
        8 * 1024 * 1024
    );
    drop(live);

    let verified =
        LiveRevisionStore::open(&canonical, &directory.path().join("independent-cache")).unwrap();
    assert_eq!(
        verified.store().asset(original_id).unwrap().unwrap(),
        vec![0x5a; 8 * 1024 * 1024]
    );
    assert!(
        fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().contains("publish"))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linked_canonical_falls_back_without_changing_the_external_alias() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let mut fixture = Fixture::new();
    let alias = fixture.directory.path().join("external-alias.prism");
    fs::set_permissions(&fixture.canonical, fs::Permissions::from_mode(0o640)).unwrap();
    fs::hard_link(&fixture.canonical, &alias).unwrap();
    let alias_bytes = fs::read(&alias).unwrap();
    let alias_mode = alias.metadata().unwrap().mode();

    let first = fixture.append(fixture.human_session, fixture.root, "Linked fallback");
    assert_eq!(
        fixture.live.last_publish_stats().strategy,
        PublishStrategy::FullCopy
    );
    assert!(!fixture.live.last_publish_stats().incremental);
    assert_eq!(fs::read(&alias).unwrap(), alias_bytes);
    assert_eq!(alias.metadata().unwrap().mode(), alias_mode);
    assert_eq!(fixture.canonical.metadata().unwrap().nlink(), 1);

    fixture.append(fixture.human_session, first, "After linked fallback");
    assert_eq!(
        fixture.live.last_publish_stats().strategy,
        PublishStrategy::PageDiffExchange
    );
    assert_eq!(fs::read(&alias).unwrap(), alias_bytes);
    assert_eq!(alias.metadata().unwrap().mode(), alias_mode);
}

#[cfg(target_os = "linux")]
#[test]
fn distinct_cache_publishers_detect_same_generation_peer_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("conflict.lumen");
    let session = SessionId::new();
    let (mut first, info) = LiveRevisionStore::create(
        &canonical,
        &directory.path().join("cache-a"),
        NewProject {
            application_id: "spectrum.test".into(),
            application_version: "1.0.0".into(),
            actor: actor("person:1", ActorKind::Human),
            session_id: session,
            root_label: Some("Created".into()),
            track_kind: "test.document".into(),
            track_label: "Document".into(),
            initial_snapshots: vec![payload("test.snapshot", b"root")],
            assets: Vec::new(),
        },
    )
    .unwrap();
    let mut second =
        LiveRevisionStore::open(&canonical, &directory.path().join("cache-b")).unwrap();

    let first_revision = first
        .mutate(|store| {
            store.append(AppendRevision {
                track_id: info.default_track_id,
                session_id: session,
                expected_parent: info.root_revision,
                application_version: "1.0.0".into(),
                label: Some("First peer".into()),
                command_count: 1,
                operation_payloads: vec![payload("test.operations", b"first")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
        })
        .unwrap()
        .id;
    let second_revision = second
        .mutate(|store| {
            store.append(AppendRevision {
                track_id: info.default_track_id,
                session_id: session,
                expected_parent: info.root_revision,
                application_version: "1.0.0".into(),
                label: Some("Second peer".into()),
                command_count: 1,
                operation_payloads: vec![payload("test.operations", b"second")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
        })
        .unwrap()
        .id;
    assert!(second.pending_publish_error().is_some());
    assert!(second.store().revision(second_revision).unwrap().is_some());
    drop(first);
    drop(second);

    let verified =
        LiveRevisionStore::open(&canonical, &directory.path().join("verified-cache")).unwrap();
    assert!(verified.store().revision(first_revision).unwrap().is_some());
    assert!(
        verified
            .store()
            .revision(second_revision)
            .unwrap()
            .is_none()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn read_only_canonical_with_an_inactive_wal_is_prepared_without_chmod() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let project_id = fixture.live.store().project_info().unwrap().project_id;
    drop(fixture.live);
    fs::set_permissions(&fixture.canonical, fs::Permissions::from_mode(0o400)).unwrap();
    fs::write(sidecar(&fixture.canonical, "-wal"), []).unwrap();

    let reopened = LiveRevisionStore::open(
        &fixture.canonical,
        &fixture.directory.path().join("read-only-reopen-cache"),
    )
    .unwrap();
    assert_eq!(
        fixture.canonical.metadata().unwrap().permissions().mode() & 0o777,
        0o400
    );
    assert_eq!(
        reopened.store().project_info().unwrap().project_id,
        project_id
    );
}

#[cfg(target_os = "linux")]
#[test]
fn cross_filesystem_cache_uses_the_full_copy_fallback() {
    use std::os::unix::fs::MetadataExt;

    let shared_memory = Path::new("/dev/shm");
    if !shared_memory.is_dir() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir_in(shared_memory).unwrap();
    if directory.path().metadata().unwrap().dev() == cache.path().metadata().unwrap().dev() {
        return;
    }
    let canonical = directory.path().join("cross-filesystem.lumen");
    let session = SessionId::new();
    let (mut live, info) = LiveRevisionStore::create(
        &canonical,
        cache.path(),
        NewProject {
            application_id: "spectrum.test".into(),
            application_version: "1.0.0".into(),
            actor: actor("person:1", ActorKind::Human),
            session_id: session,
            root_label: Some("Created".into()),
            track_kind: "test.document".into(),
            track_label: "Document".into(),
            initial_snapshots: vec![payload("test.snapshot", b"root")],
            assets: Vec::new(),
        },
    )
    .unwrap();
    let revision = live
        .mutate(|store| {
            store.append(AppendRevision {
                track_id: info.default_track_id,
                session_id: session,
                expected_parent: info.root_revision,
                application_version: "1.0.0".into(),
                label: Some("Fallback".into()),
                command_count: 1,
                operation_payloads: vec![payload("test.operations", b"fallback")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
        })
        .unwrap()
        .id;
    assert!(!live.last_publish_stats().incremental);
    drop(live);
    let verified =
        LiveRevisionStore::open(&canonical, &directory.path().join("verified-cache")).unwrap();
    assert!(verified.store().revision(revision).unwrap().is_some());
}

#[cfg(not(target_os = "linux"))]
#[test]
fn non_linux_publish_keeps_the_atomic_full_copy_fallback() {
    let mut fixture = Fixture::new();
    fixture.append(fixture.human_session, fixture.root, "Fallback");
    let stats = fixture.live.last_publish_stats();
    assert!(!stats.incremental);
    assert_eq!(
        stats.written_bytes,
        fixture.canonical.metadata().unwrap().len()
    );
}

fn actor(id: &str, kind: ActorKind) -> Actor {
    Actor {
        id: id.into(),
        display_name: id.into(),
        kind,
    }
}

fn payload(family: &str, bytes: &[u8]) -> Payload {
    Payload::new(Encoding::new(family, 1), bytes.to_vec())
}

fn sidecar(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut value: OsString = path.as_os_str().to_owned();
    value.push(suffix);
    value.into()
}
