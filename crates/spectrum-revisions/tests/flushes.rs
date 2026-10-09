//! Disk flushes are slow on macOS (each one flushes the whole drive), so a
//! store flushes only after it was written. Its own test process, so no other
//! test's flushes land in the counts.
use spectrum_revisions::{
    Actor, ActorKind, AppendRevision, Encoding, NewProject, Payload, RevisionStore, SessionId,
    io_stats,
};

#[test]
fn a_store_flushes_after_writes_and_never_on_open_or_read() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("project.spectrum");
    let session = SessionId::new();
    let (mut store, project) = RevisionStore::create(
        &path,
        NewProject {
            application_id: "spectrum.spectrum".into(),
            application_version: "1.0.0".into(),
            actor: Actor {
                id: "person:1".into(),
                display_name: "Person".into(),
                kind: ActorKind::Human,
            },
            session_id: session,
            root_label: None,
            track_kind: "test.document".into(),
            track_label: "Document".into(),
            initial_snapshots: vec![payload(b"root")],
            assets: Vec::new(),
        },
    )
    .unwrap();

    let before = io_stats();
    store
        .append(AppendRevision {
            track_id: project.default_track_id,
            session_id: session,
            expected_parent: project.root_revision,
            application_version: "1.0.0".into(),
            label: None,
            command_count: 1,
            operation_payloads: vec![payload(b"edit")],
            snapshots: Vec::new(),
            assets: Vec::new(),
        })
        .unwrap();
    assert!(
        io_stats().since(before).syncs > 0,
        "a write must be flushed"
    );
    drop(store);

    let before = io_stats();
    let store = RevisionStore::open(&path).unwrap();
    store.checkpoint().unwrap();
    store.project_info().unwrap();
    store.checkpoint().unwrap();
    drop(store);
    assert_eq!(
        io_stats().since(before).syncs,
        0,
        "opening and reading flushed"
    );
}

fn payload(bytes: &[u8]) -> Payload {
    Payload::new(Encoding::new("test.document", 1), bytes.to_vec())
}
