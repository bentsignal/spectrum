use std::fs;

use spectrum_revisions::{
    Actor, ActorKind, AppendRevision, CollaborationMode, CollaborationStatus, CollaborationSync,
    Compatibility, Encoding, NewProject, NewTrack, Payload, Preview, RevisionError, RevisionId,
    RevisionStore, SessionId, TrackId,
};
use tempfile::TempDir;

struct Fixture {
    directory: TempDir,
    store: RevisionStore,
    root: RevisionId,
    track: TrackId,
    human_session: SessionId,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let human_session = SessionId::new();
        let (store, project) = RevisionStore::create(
            &directory.path().join("project.prism"),
            NewProject {
                application_id: "spectrum.prism".into(),
                application_version: "1.0.0".into(),
                actor: human("person:1", "Person"),
                session_id: human_session,
                root_label: Some("Created project".into()),
                track_kind: "prism.document".into(),
                track_label: "Document".into(),
                initial_snapshots: vec![payload("prism.document", 1, b"root")],
                assets: Vec::new(),
            },
        )
        .unwrap();
        Self {
            directory,
            store,
            root: project.root_revision,
            track: project.default_track_id,
            human_session,
        }
    }

    fn append(&mut self, session_id: SessionId, parent: RevisionId, label: &str) -> RevisionId {
        self.store
            .append(AppendRevision {
                track_id: self.track,
                session_id,
                expected_parent: parent,
                application_version: "1.0.0".into(),
                label: Some(label.into()),
                command_count: 1,
                operation_payloads: vec![payload("prism.commands", 1, label.as_bytes())],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
            .unwrap()
            .id
    }
}

struct V1Compatibility;

impl Compatibility for V1Compatibility {
    fn supports_snapshot(&self, encoding: &Encoding) -> bool {
        encoding.family == "prism.document"
            && encoding.version <= 1
            && encoding.required_capabilities.is_empty()
    }

    fn supports_operations(&self, encoding: &Encoding) -> bool {
        encoding.family == "prism.commands"
            && encoding.version <= 1
            && encoding.required_capabilities.is_empty()
    }
}

#[test]
fn creates_one_portable_revision_store_and_reopens_it() {
    let mut fixture = Fixture::new();
    let path = fixture.directory.path().join("project.prism");
    let project_id = fixture.store.project_info().unwrap().project_id;
    let latest = fixture.append(fixture.human_session, fixture.root, "Portable edit");

    let portable_copy = fixture.directory.path().join("portable-copy.prism");
    fs::copy(&path, &portable_copy).unwrap();
    let copied = RevisionStore::open(&portable_copy).unwrap();
    assert_eq!(copied.project_info().unwrap().project_id, project_id);
    drop(copied);
    drop(fixture.store);

    assert!(path.is_file());
    assert!(!path.with_extension("prism-wal").exists());
    let reopened = RevisionStore::open(&path).unwrap();
    let info = reopened.project_info().unwrap();
    assert_eq!(info.project_id, project_id);
    assert_eq!(info.application_id, "spectrum.prism");
    assert_eq!(info.container_format, 3);
    assert_eq!(info.root_revision, fixture.root);
    assert_eq!(
        reopened
            .session(fixture.human_session)
            .unwrap()
            .unwrap()
            .cursor,
        latest
    );
}

#[test]
fn existing_revision_containers_gain_collaboration_metadata_on_open() {
    let fixture = Fixture::new();
    let path = fixture.directory.path().join("project.prism");
    let human_session = fixture.human_session;
    fixture.store.checkpoint().unwrap();
    drop(fixture.store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("DROP TABLE collaborations;")
        .unwrap();
    drop(connection);

    let mut reopened = RevisionStore::open(&path).unwrap();
    let collaboration = reopened
        .start_collaboration(
            human_session,
            reopened.project_info().unwrap().default_track_id,
            Actor {
                id: "agent:migrated".into(),
                display_name: "Migrated Agent".into(),
                kind: ActorKind::Agent,
            },
            CollaborationMode::Separate,
        )
        .unwrap();
    assert_eq!(collaboration.source_session, human_session);
}

#[test]
fn independent_connections_can_extend_the_same_project() {
    let mut fixture = Fixture::new();
    let path = fixture.directory.path().join("project.prism");
    let mut agent_store = RevisionStore::open(&path).unwrap();
    let agent_session = SessionId::new();
    agent_store
        .resume_session(
            agent_session,
            Actor {
                id: "agent:1".into(),
                display_name: "Agent".into(),
                kind: ActorKind::Agent,
            },
            fixture.root,
        )
        .unwrap();

    let human_child = fixture.append(fixture.human_session, fixture.root, "Human move");
    let agent_child = agent_store
        .append(AppendRevision {
            track_id: fixture.track,
            session_id: agent_session,
            expected_parent: fixture.root,
            application_version: "1.0.0".into(),
            label: Some("Agent text edit".into()),
            command_count: 1,
            operation_payloads: vec![payload("prism.commands", 1, b"agent")],
            snapshots: Vec::new(),
            assets: Vec::new(),
        })
        .unwrap()
        .id;

    let children = fixture.store.children(fixture.root).unwrap();
    assert_eq!(children.len(), 2);
    assert!(children.iter().any(|revision| revision.id == human_child));
    assert!(children.iter().any(|revision| revision.id == agent_child));
}

#[test]
fn together_collaboration_follows_until_the_human_edits() {
    let mut fixture = Fixture::new();
    let collaboration = fixture
        .store
        .start_collaboration(
            fixture.human_session,
            fixture.track,
            Actor {
                id: "agent:codex".into(),
                display_name: "Codex".into(),
                kind: ActorKind::Agent,
            },
            CollaborationMode::Together,
        )
        .unwrap();
    assert_eq!(collaboration.base_revision, fixture.root);

    let agent_first = fixture.append(collaboration.agent_session, fixture.root, "Agent first");
    assert!(matches!(
        fixture.store.sync_together(fixture.human_session).unwrap(),
        CollaborationSync::Advanced { from, to, .. }
            if from == fixture.root && to == agent_first
    ));
    assert_eq!(
        fixture
            .store
            .session(fixture.human_session)
            .unwrap()
            .unwrap()
            .cursor,
        agent_first
    );

    let human_child = fixture.append(fixture.human_session, agent_first, "Human continued");
    let agent_child = fixture.append(collaboration.agent_session, agent_first, "Agent continued");
    let sync = fixture.store.sync_together(fixture.human_session).unwrap();
    assert!(matches!(
        sync,
        CollaborationSync::Split(ref collaboration)
            if collaboration.status == CollaborationStatus::Split
    ));
    assert_eq!(
        fixture
            .store
            .session(fixture.human_session)
            .unwrap()
            .unwrap()
            .cursor,
        human_child
    );
    assert_eq!(
        fixture
            .store
            .session(collaboration.agent_session)
            .unwrap()
            .unwrap()
            .cursor,
        agent_child
    );
}

#[test]
fn separate_collaboration_never_moves_the_human_session() {
    let mut fixture = Fixture::new();
    let collaboration = fixture
        .store
        .start_collaboration(
            fixture.human_session,
            fixture.track,
            Actor {
                id: "agent:separate".into(),
                display_name: "Separate Agent".into(),
                kind: ActorKind::Agent,
            },
            CollaborationMode::Separate,
        )
        .unwrap();
    fixture.append(
        collaboration.agent_session,
        fixture.root,
        "Independent idea",
    );
    assert_eq!(
        fixture.store.sync_together(fixture.human_session).unwrap(),
        CollaborationSync::Idle
    );
    assert_eq!(
        fixture
            .store
            .session(fixture.human_session)
            .unwrap()
            .unwrap()
            .cursor,
        fixture.root
    );
}

#[test]
fn collaboration_on_one_track_ignores_human_edits_on_another_track() {
    let mut fixture = Fixture::new();
    let second = fixture
        .store
        .create_tracks(
            fixture.human_session,
            "1.0.0",
            vec![NewTrack {
                kind: "lumen.photo".into(),
                label: "photo:2".into(),
                root_label: Some("Imported second photo".into()),
                initial_snapshots: vec![payload("photo.snapshot", 1, b"photo-two")],
                assets: Vec::new(),
            }],
        )
        .unwrap()
        .remove(0);
    let collaboration = fixture
        .store
        .start_collaboration(
            fixture.human_session,
            fixture.track,
            Actor {
                id: "agent:photo-one".into(),
                display_name: "Photo Agent".into(),
                kind: ActorKind::Agent,
            },
            CollaborationMode::Together,
        )
        .unwrap();

    let human_second = fixture
        .store
        .append(AppendRevision {
            track_id: second.id,
            session_id: fixture.human_session,
            expected_parent: second.root_revision,
            application_version: "1.0.0".into(),
            label: Some("Human edits photo two".into()),
            command_count: 1,
            operation_payloads: vec![payload("photo.operations", 1, b"human-two")],
            snapshots: Vec::new(),
            assets: Vec::new(),
        })
        .unwrap()
        .id;
    let agent_first = fixture.append(
        collaboration.agent_session,
        fixture.root,
        "Agent edits photo one",
    );

    assert!(matches!(
        fixture.store.sync_together(fixture.human_session).unwrap(),
        CollaborationSync::Advanced { to, .. } if to == agent_first
    ));
    assert_eq!(
        fixture
            .store
            .session_on_track(fixture.human_session, fixture.track)
            .unwrap()
            .unwrap()
            .cursor,
        agent_first
    );
    assert_eq!(
        fixture
            .store
            .session_on_track(fixture.human_session, second.id)
            .unwrap()
            .unwrap()
            .cursor,
        human_second
    );
}

#[test]
fn multi_track_append_is_atomic_when_one_cursor_is_stale() {
    let mut fixture = Fixture::new();
    let second = fixture
        .store
        .create_tracks(
            fixture.human_session,
            "1.0.0",
            vec![NewTrack {
                kind: "lumen.photo".into(),
                label: "photo:2".into(),
                root_label: Some("Second".into()),
                initial_snapshots: vec![payload("photo.snapshot", 1, b"second")],
                assets: Vec::new(),
            }],
        )
        .unwrap()
        .remove(0);
    let current = fixture.append(fixture.human_session, fixture.root, "Already advanced");
    let error = fixture
        .store
        .append_batch(vec![
            AppendRevision {
                track_id: second.id,
                session_id: fixture.human_session,
                expected_parent: second.root_revision,
                application_version: "1.0.0".into(),
                label: Some("Would edit second".into()),
                command_count: 1,
                operation_payloads: vec![payload("photo.operations", 1, b"second-edit")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            },
            AppendRevision {
                track_id: fixture.track,
                session_id: fixture.human_session,
                expected_parent: fixture.root,
                application_version: "1.0.0".into(),
                label: Some("Stale first".into()),
                command_count: 1,
                operation_payloads: vec![payload("prism.commands", 1, b"stale")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            },
        ])
        .unwrap_err();
    assert!(matches!(error, RevisionError::CursorMoved { actual, .. } if actual == current));
    assert!(
        fixture
            .store
            .children(second.root_revision)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn multi_track_append_records_one_shared_change_set() {
    let mut fixture = Fixture::new();
    let second = fixture
        .store
        .create_tracks(
            fixture.human_session,
            "1.0.0",
            vec![NewTrack {
                kind: "lumen.photo".into(),
                label: "photo:2".into(),
                root_label: Some("Second".into()),
                initial_snapshots: vec![payload("photo.snapshot", 1, b"second")],
                assets: Vec::new(),
            }],
        )
        .unwrap()
        .remove(0);
    let revisions = fixture
        .store
        .append_batch(vec![
            AppendRevision {
                track_id: fixture.track,
                session_id: fixture.human_session,
                expected_parent: fixture.root,
                application_version: "1.0.0".into(),
                label: Some("Batch edit".into()),
                command_count: 1,
                operation_payloads: vec![payload("photo.operations", 1, b"first")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            },
            AppendRevision {
                track_id: second.id,
                session_id: fixture.human_session,
                expected_parent: second.root_revision,
                application_version: "1.0.0".into(),
                label: Some("Batch edit".into()),
                command_count: 1,
                operation_payloads: vec![payload("photo.operations", 1, b"second")],
                snapshots: Vec::new(),
                assets: Vec::new(),
            },
        ])
        .unwrap();
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0].change_set_id, revisions[1].change_set_id);
}

#[test]
fn sessions_create_branches_without_a_global_head_conflict() {
    let mut fixture = Fixture::new();
    let human_child = fixture.append(fixture.human_session, fixture.root, "Human move");
    let agent_session = SessionId::new();
    fixture
        .store
        .resume_session(
            agent_session,
            Actor {
                id: "agent:1".into(),
                display_name: "Agent".into(),
                kind: ActorKind::Agent,
            },
            fixture.root,
        )
        .unwrap();
    let agent_child = fixture.append(agent_session, fixture.root, "Agent text edit");

    let children = fixture.store.children(fixture.root).unwrap();
    assert_eq!(children.len(), 2);
    assert!(children.iter().any(|revision| revision.id == human_child));
    assert!(children.iter().any(|revision| revision.id == agent_child));
    assert_eq!(
        fixture
            .store
            .session(fixture.human_session)
            .unwrap()
            .unwrap()
            .cursor,
        human_child
    );
    assert_eq!(
        fixture
            .store
            .session(agent_session)
            .unwrap()
            .unwrap()
            .cursor,
        agent_child
    );
    let revisions = fixture.store.revisions().unwrap();
    assert_eq!(revisions.len(), 3);
    assert_eq!(revisions[0].id, fixture.root);
    let sessions = fixture.store.sessions().unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().any(|session| session.cursor == human_child));
    assert!(sessions.iter().any(|session| session.cursor == agent_child));
}

#[test]
fn navigation_creates_no_revision_and_next_action_forks() {
    let mut fixture = Fixture::new();
    let first = fixture.append(fixture.human_session, fixture.root, "First");
    let second = fixture.append(fixture.human_session, first, "Second");
    fixture
        .store
        .move_session(fixture.human_session, second, first)
        .unwrap();
    assert_eq!(fixture.store.children(first).unwrap().len(), 1);

    let alternate = fixture.append(fixture.human_session, first, "Alternate");
    let children = fixture.store.children(first).unwrap();
    assert_eq!(children.len(), 2);
    assert!(children.iter().any(|revision| revision.id == second));
    assert!(children.iter().any(|revision| revision.id == alternate));
}

#[test]
fn stale_session_writes_fail_without_silently_overwriting() {
    let mut fixture = Fixture::new();
    let current = fixture.append(fixture.human_session, fixture.root, "Current");
    let error = fixture
        .store
        .append(AppendRevision {
            track_id: fixture.track,
            session_id: fixture.human_session,
            expected_parent: fixture.root,
            application_version: "1.0.0".into(),
            label: Some("Stale".into()),
            command_count: 1,
            operation_payloads: vec![payload("prism.commands", 1, b"stale")],
            snapshots: Vec::new(),
            assets: Vec::new(),
        })
        .unwrap_err();
    assert!(matches!(
        error,
        RevisionError::CursorMoved { actual, .. } if actual == current
    ));
    assert_eq!(fixture.store.children(fixture.root).unwrap().len(), 1);
}

#[test]
fn failed_multi_encoding_insert_rolls_back_the_entire_revision() {
    let mut fixture = Fixture::new();
    let duplicate = payload("prism.commands", 1, b"same");
    let error = fixture
        .store
        .append(AppendRevision {
            track_id: fixture.track,
            session_id: fixture.human_session,
            expected_parent: fixture.root,
            application_version: "1.0.0".into(),
            label: Some("Must roll back".into()),
            command_count: 1,
            operation_payloads: vec![duplicate.clone(), duplicate],
            snapshots: Vec::new(),
            assets: Vec::new(),
        })
        .unwrap_err();
    assert!(matches!(error, RevisionError::Database(_)));
    assert!(fixture.store.children(fixture.root).unwrap().is_empty());
    assert_eq!(
        fixture
            .store
            .session(fixture.human_session)
            .unwrap()
            .unwrap()
            .cursor,
        fixture.root
    );
}

#[test]
fn compatible_snapshots_bridge_commands_an_old_app_cannot_decode() {
    let mut fixture = Fixture::new();
    let first = fixture.append(fixture.human_session, fixture.root, "V1 action");
    let second = fixture
        .store
        .append(AppendRevision {
            track_id: fixture.track,
            session_id: fixture.human_session,
            expected_parent: first,
            application_version: "2.0.0".into(),
            label: Some("V2 action".into()),
            command_count: 1,
            operation_payloads: vec![Payload::new(
                Encoding::new("prism.commands", 2).requiring("live_effects"),
                b"v2".to_vec(),
            )],
            snapshots: Vec::new(),
            assets: Vec::new(),
        })
        .unwrap()
        .id;

    assert!(matches!(
        fixture.store.replay_plan(second, &V1Compatibility),
        Err(RevisionError::IncompatibleRevision(id)) if id == second
    ));
    assert_eq!(
        fixture
            .store
            .newest_compatible_ancestor(second, &V1Compatibility)
            .unwrap(),
        first
    );

    fixture
        .store
        .add_snapshot(second, payload("prism.document", 1, b"v1-compatible-state"))
        .unwrap();
    let plan = fixture.store.replay_plan(second, &V1Compatibility).unwrap();
    assert_eq!(plan.snapshot_revision, second);
    assert_eq!(plan.snapshot.bytes, b"v1-compatible-state");
    assert!(plan.steps.is_empty());
}

#[test]
fn replay_uses_the_nearest_snapshot_and_only_the_short_tail() {
    let mut fixture = Fixture::new();
    let first = fixture.append(fixture.human_session, fixture.root, "First");
    fixture
        .store
        .add_snapshot(first, payload("prism.document", 1, b"at-first"))
        .unwrap();
    let second = fixture.append(fixture.human_session, first, "Second");
    let plan = fixture.store.replay_plan(second, &V1Compatibility).unwrap();
    assert_eq!(plan.snapshot_revision, first);
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].revision.id, second);
}

#[test]
fn assets_are_content_addressed_and_integrity_checked() {
    let mut fixture = Fixture::new();
    let bytes = vec![42; 1024 * 1024];
    let first = fixture.store.put_asset("image/raw", &bytes).unwrap();
    let second = fixture.store.put_asset("image/raw", &bytes).unwrap();
    assert_eq!(first, second);
    assert_eq!(fixture.store.asset(first).unwrap().unwrap(), bytes);

    fixture
        .store
        .put_preview(&Preview {
            revision_id: fixture.root,
            format: "image/webp".into(),
            width: 320,
            height: 180,
            bytes: b"preview".to_vec(),
        })
        .unwrap();
    fixture.store.verify_integrity().unwrap();
}

#[test]
fn tampered_payload_is_reported_as_corruption() {
    let mut fixture = Fixture::new();
    let revision = fixture.append(fixture.human_session, fixture.root, "Action");
    let path = fixture.directory.path().join("project.prism");
    fixture.store.checkpoint().unwrap();
    drop(fixture.store);

    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE operation_payloads SET bytes = ?1 WHERE revision_id = ?2",
            rusqlite::params![b"tampered", revision.as_bytes().as_slice()],
        )
        .unwrap();
    drop(connection);

    let store = RevisionStore::open(&path).unwrap();
    assert!(matches!(
        store.verify_integrity(),
        Err(RevisionError::Corrupt(message)) if message.contains("payload hash mismatch")
    ));
}

#[test]
fn arbitrary_files_are_not_mistaken_for_revision_stores() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.prism");
    fs::write(&path, br#"{"version":1}"#).unwrap();
    assert!(RevisionStore::open(&path).is_err());
}

fn human(id: &str, name: &str) -> Actor {
    Actor {
        id: id.into(),
        display_name: name.into(),
        kind: ActorKind::Human,
    }
}

fn payload(family: &str, version: u32, bytes: &[u8]) -> Payload {
    Payload::new(Encoding::new(family, version), bytes.to_vec())
}
