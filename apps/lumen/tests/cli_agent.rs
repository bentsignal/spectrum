use std::{io::Write, path::Path, process::Command as ProcessCommand, str::FromStr};

use flate2::{Compression, write::ZlibEncoder};
use image::{Rgba, RgbaImage};
use lumen_core::{AdjustmentPatch, Command, Project, Workspace};
use serde_json::Value;
use spectrum_revisions::{
    Actor, ActorKind, Asset, CollaborationMode, CollaborationStatus, CollaborationSync, Encoding,
    NewProject, Payload, RevisionStore, SessionId,
};

#[test]
fn cli_agent_sessions_support_together_and_separate_workflows() {
    let human_session = SessionId::new();
    let directory = std::env::temp_dir().join(format!("lumen-cli-agent-{human_session}"));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    RgbaImage::from_pixel(4, 4, Rgba([30, 80, 140, 255]))
        .save(&source)
        .unwrap();
    let second_source = directory.join("second.png");
    RgbaImage::from_pixel(4, 4, Rgba([120, 50, 20, 255]))
        .save(&second_source)
        .unwrap();
    let project_path = directory.join("cli-agent.lumen");
    let mut project = Project::new("CLI collaboration");
    project.import(&[source, second_source]).unwrap();
    let mut human = Workspace::create_durable(
        project,
        &project_path,
        Actor {
            id: "person:lumen-cli-test".into(),
            display_name: "Lumen CLI Test User".into(),
            kind: ActorKind::Human,
        },
        human_session,
    )
    .unwrap();

    let together = run_lumen(&[
        "--catalog",
        project_path.to_str().unwrap(),
        "agent",
        "start",
        "1",
        "--mode",
        "together",
        "--name",
        "Codex Test",
    ]);
    assert_eq!(together["mode"], "together");
    let together_session = session(&together);
    run_lumen(&[
        "--catalog",
        project_path.to_str().unwrap(),
        "--session",
        &together_session.to_string(),
        "edit",
        "1",
        "--exposure",
        "1.25",
    ]);
    assert!(matches!(
        human.sync_together().unwrap(),
        CollaborationSync::Advanced { .. }
    ));
    assert_eq!(human.project.photo(1).unwrap().adjustments.exposure, 1.25);
    assert_eq!(
        human.project.photo(1).unwrap().history.len(),
        1,
        "durable photo tracks replace bounded in-object snapshots"
    );

    human
        .execute(Command::Adjust {
            id: 2,
            patch: AdjustmentPatch {
                contrast: Some(31.0),
                ..Default::default()
            },
        })
        .unwrap();
    run_lumen(&[
        "--catalog",
        project_path.to_str().unwrap(),
        "--session",
        &together_session.to_string(),
        "edit",
        "1",
        "--temperature",
        "8",
    ]);
    assert!(matches!(
        human.sync_together().unwrap(),
        CollaborationSync::Advanced { .. }
    ));
    assert_eq!(human.project.photo(1).unwrap().adjustments.temperature, 8.0);
    assert_eq!(human.project.photo(2).unwrap().adjustments.contrast, 31.0);
    assert_eq!(human.history_for(1).unwrap().unwrap().revisions.len(), 3);
    assert_eq!(human.history_for(2).unwrap().unwrap().revisions.len(), 2);

    human
        .execute(Command::Adjust {
            id: 1,
            patch: AdjustmentPatch {
                contrast: Some(14.0),
                ..Default::default()
            },
        })
        .unwrap();
    run_lumen(&[
        "--catalog",
        project_path.to_str().unwrap(),
        "--session",
        &together_session.to_string(),
        "edit",
        "1",
        "--temperature",
        "18",
    ]);
    assert!(matches!(
        human.sync_together().unwrap(),
        CollaborationSync::Split(_)
    ));
    let together_status = status(&project_path, together_session);
    assert_eq!(
        together_status["collaboration"]["status"],
        serde_json::to_value(CollaborationStatus::Split).unwrap()
    );

    let separate = run_lumen(&[
        "--catalog",
        project_path.to_str().unwrap(),
        "agent",
        "start",
        "1",
        "--mode",
        "separate",
        "--name",
        "Claude Test",
    ]);
    assert_eq!(
        separate["mode"],
        serde_json::to_value(CollaborationMode::Separate).unwrap()
    );
    let separate_session = session(&separate);
    run_lumen(&[
        "--catalog",
        project_path.to_str().unwrap(),
        "--session",
        &separate_session.to_string(),
        "edit",
        "1",
        "--saturation",
        "22",
    ]);
    assert_eq!(human.sync_together().unwrap(), CollaborationSync::Idle);
    assert_eq!(human.project.photo(1).unwrap().adjustments.saturation, 0.0);
    drop(human);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn legacy_catalog_migrates_without_replacing_the_source() {
    let session = SessionId::new();
    let directory = std::env::temp_dir().join(format!("lumen-legacy-migration-{session}"));
    std::fs::create_dir_all(&directory).unwrap();
    let legacy = directory.join("legacy.lumencatalog");
    Project::new("Legacy").save(&legacy).unwrap();
    let workspace = Workspace::open_as(
        &legacy,
        Actor {
            id: "person:migration-test".into(),
            display_name: "Migration Test".into(),
            kind: ActorKind::Human,
        },
        session,
    )
    .unwrap();
    assert!(legacy.exists());
    let migrated = directory.join("legacy.lumen");
    assert_eq!(workspace.catalog_path.as_deref(), Some(migrated.as_path()));
    assert!(workspace.is_durable());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn earlier_project_wide_lumen_files_upgrade_to_photo_tracks() {
    let original_session = SessionId::new();
    let directory = std::env::temp_dir().join(format!("lumen-track-upgrade-{original_session}"));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    RgbaImage::from_pixel(4, 4, Rgba([80, 100, 130, 255]))
        .save(&source)
        .unwrap();
    let mut project = Project::new("Earlier durable project");
    project.import(std::slice::from_ref(&source)).unwrap();
    project.photo_mut(1).unwrap().adjustments.exposure = 0.75;
    let bytes = std::fs::read(&source).unwrap();
    let asset = Asset::new("image/png", bytes);
    project.photo_mut(1).unwrap().path = format!("spectrum-asset:{}.png", asset.id).into();
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&serde_json::to_vec(&project).unwrap())
        .unwrap();
    let snapshot = encoder.finish().unwrap();
    let path = directory.join("earlier.lumen");
    let (store, _) = RevisionStore::create(
        &path,
        NewProject {
            application_id: "spectrum.lumen".into(),
            application_version: "0.5.0".into(),
            actor: Actor {
                id: "person:earlier".into(),
                display_name: "Earlier User".into(),
                kind: ActorKind::Human,
            },
            session_id: original_session,
            root_label: Some("Created project".into()),
            track_kind: "spectrum.lumen.project".into(),
            track_label: "Project".into(),
            initial_snapshots: vec![Payload::new(
                Encoding::new("spectrum.lumen.project", 1).requiring("deflate"),
                snapshot,
            )],
            assets: vec![asset],
        },
    )
    .unwrap();
    store.checkpoint().unwrap();
    drop(store);

    let workspace = Workspace::open_as(
        &path,
        Actor {
            id: "person:upgraded".into(),
            display_name: "Upgraded User".into(),
            kind: ActorKind::Human,
        },
        SessionId::new(),
    )
    .unwrap();
    assert_eq!(
        workspace.project.photo(1).unwrap().adjustments.exposure,
        0.75
    );
    assert!(workspace.project.photo(1).unwrap().path.is_file());
    assert_eq!(
        workspace.history_for(1).unwrap().unwrap().revisions.len(),
        1
    );
    std::fs::remove_dir_all(directory).unwrap();
}

fn status(project: &Path, session: SessionId) -> Value {
    run_lumen(&[
        "--catalog",
        project.to_str().unwrap(),
        "--session",
        &session.to_string(),
        "agent",
        "status",
    ])
}

fn session(output: &Value) -> SessionId {
    SessionId::from_str(output["session"].as_str().unwrap()).unwrap()
}

fn run_lumen(arguments: &[&str]) -> Value {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_lumen"))
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "lumen failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
