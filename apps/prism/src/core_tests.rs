use super::*;
use image::{Rgba, RgbaImage};
use std::{
    ffi::OsString,
    time::{SystemTime, UNIX_EPOCH},
};

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-{label}-{stamp}"))
}

fn test_actor(id: &str, kind: spectrum_revisions::ActorKind) -> spectrum_revisions::Actor {
    spectrum_revisions::Actor {
        id: id.into(),
        display_name: id.into(),
        kind,
    }
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_owned();
    value.push(suffix);
    value.into()
}

#[test]
fn command_history_restores_layers() {
    let mut workspace = Workspace::default();
    workspace
        .execute(Command::AddRectangle {
            name: Some("Card".into()),
            width: 100,
            height: 80,
            color: [255, 0, 0, 255],
            corner_radius: 8.0,
            x: 20.0,
            y: 30.0,
        })
        .unwrap();
    assert_eq!(workspace.document.layers.len(), 1);
    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.layers.is_empty());
    workspace.execute(Command::Redo).unwrap();
    assert_eq!(workspace.document.layers.len(), 1);
}

#[test]
fn clipping_uses_layer_below_alpha() {
    let mut document = Document::new("Clip", 20, 20);
    let mut workspace = Workspace::new(document.clone(), None);
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 5,
            height: 5,
            color: [255, 255, 255, 255],
            corner_radius: 0.0,
            x: 2.0,
            y: 2.0,
        })
        .unwrap();
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 20,
            height: 20,
            color: [255, 0, 0, 255],
            corner_radius: 0.0,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    let top = workspace.document.selected.unwrap();
    workspace
        .execute(Command::SetClipping {
            id: top,
            enabled: true,
        })
        .unwrap();
    document = workspace.document;
    document.background = [0, 0, 0, 0];
    let rendered = render_document(&document, None).unwrap().to_rgba8();
    assert_eq!(rendered.get_pixel(3, 3)[0], 255);
    assert_eq!(rendered.get_pixel(10, 10)[3], 0);
}

#[test]
fn text_and_shapes_render() {
    let mut workspace = Workspace::new(Document::new("Poster", 400, 200), None);
    workspace
        .execute(Command::AddText {
            text: "Prism".into(),
            name: None,
            font_size: 48.0,
            color: [255, 255, 255, 255],
            x: 30.0,
            y: 40.0,
        })
        .unwrap();
    let rendered = render_document(&workspace.document, None).unwrap();
    assert_eq!((rendered.width(), rendered.height()), (400, 200));
}

#[test]
fn automatic_text_names_follow_content_without_overwriting_manual_names() {
    let mut workspace = Workspace::new(Document::new("Names", 400, 200), None);
    workspace
        .execute(Command::AddText {
            text: "Title\n".into(),
            name: None,
            font_size: 48.0,
            color: [255, 255, 255, 255],
            x: 30.0,
            y: 40.0,
        })
        .unwrap();
    let id = workspace.document.selected.unwrap();
    assert_eq!(workspace.document.layer(id).unwrap().name, "Title");
    workspace
        .execute(Command::UpdateText {
            id,
            text: "Updated title\nSubtitle".into(),
            font_size: 48.0,
            color: [255, 255, 255, 255],
        })
        .unwrap();
    assert_eq!(workspace.document.layer(id).unwrap().name, "Updated title");
    workspace
        .execute(Command::RenameLayer {
            id,
            name: "Cover heading".into(),
        })
        .unwrap();
    workspace
        .execute(Command::UpdateText {
            id,
            text: "Final title".into(),
            font_size: 48.0,
            color: [255, 255, 255, 255],
        })
        .unwrap();
    assert_eq!(workspace.document.layer(id).unwrap().name, "Cover heading");
}

#[test]
fn text_metrics_match_the_rendered_layout() {
    let layer = Layer {
        kind: LayerKind::Text {
            text: "testing\ngy".into(),
            font_size: 72.0,
            color: [255, 255, 255, 255],
            typography: TextTypography::default(),
        },
        ..Default::default()
    };
    let rendered = render_layer_base(&layer, None).unwrap().to_rgba8();
    assert_eq!(
        (rendered.width(), rendered.height()),
        measure_text("testing\ngy", 72.0).unwrap()
    );
    assert!(rendered.pixels().any(|pixel| pixel[3] > 0));
}

#[test]
fn solid_color_preview_matches_a_uniform_layer_adjustment() {
    let adjustments = Adjustments {
        exposure: 1.25,
        contrast: 18.0,
        saturation: -12.0,
        ..Default::default()
    };
    let color = [93, 216, 199, 255];
    let preview = render_solid_color(color, &adjustments);
    let layer = Layer {
        adjustments,
        kind: LayerKind::Rectangle {
            width: 1,
            height: 1,
            color,
            corner_radius: 0.0,
        },
        ..Default::default()
    };
    let rendered = render_layer_preview(&layer, Some(1)).unwrap().to_rgba8();
    assert_eq!(preview, rendered.get_pixel(0, 0).0);
}

#[test]
fn arbitrary_rotation_never_samples_outside_source() {
    let mut workspace = Workspace::new(Document::new("Rotate", 100, 100), None);
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 37,
            height: 23,
            color: [255, 0, 0, 255],
            corner_radius: 0.0,
            x: 10.0,
            y: 10.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetTransform {
            id: 1,
            transform: Transform {
                x: 10.0,
                y: 10.0,
                rotation: 13.0,
                ..Default::default()
            },
        })
        .unwrap();
    assert!(render_document(&workspace.document, None).is_ok());
}

#[test]
fn export_refuses_to_overwrite_a_raster_source() {
    let directory = test_directory("immutable-source");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("original.png");
    RgbaImage::from_pixel(4, 4, Rgba([20, 40, 60, 255]))
        .save(&source)
        .unwrap();
    let original = fs::read(&source).unwrap();
    let mut workspace = Workspace::new(Document::new("Safety", 4, 4), None);
    workspace
        .execute(Command::AddRaster {
            path: source.clone(),
            name: None,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    assert!(export_document(&workspace.document, &source, 92).is_err());
    assert_eq!(fs::read(&source).unwrap(), original);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn non_finite_commands_are_rejected_before_serialization() {
    let mut workspace = Workspace::new(Document::new("Finite", 20, 20), None);
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 10,
            height: 10,
            color: [255, 255, 255, 255],
            corner_radius: 0.0,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    assert!(
        workspace
            .execute(Command::SetOpacity {
                id: 1,
                opacity: f32::NAN,
            })
            .is_err()
    );
    assert!(workspace.document.layer(1).unwrap().opacity.is_finite());
    assert!(
        !serde_json::to_string(&workspace.document)
            .unwrap()
            .contains("\"opacity\":null")
    );
    assert!(
        workspace
            .execute(Command::SetShapeStroke {
                id: 1,
                stroke: ShapeStroke {
                    enabled: true,
                    width: f32::NAN,
                    color: [255, 255, 255, 255],
                },
            })
            .is_err()
    );
}

#[test]
fn preview_renders_at_target_size_without_full_canvas_allocation() {
    let document = Document::new("Large", MAX_CANVAS_DIMENSION, MAX_CANVAS_DIMENSION);
    let preview = render_document(&document, Some(512)).unwrap();
    assert_eq!((preview.width(), preview.height()), (512, 512));
}

#[test]
fn selection_is_command_driven_but_not_an_undo_step() {
    let mut workspace = Workspace::new(Document::new("Selection", 20, 20), None);
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 10,
            height: 10,
            color: [255, 255, 255, 255],
            corner_radius: 0.0,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    workspace
        .execute(Command::SelectLayer { id: None })
        .unwrap();
    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.layers.is_empty());
}

#[test]
fn interaction_previews_coalesce_into_one_undo_step() {
    let mut workspace = Workspace::new(Document::new("Gesture", 400, 300), None);
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 100,
            height: 80,
            color: [255, 255, 255, 255],
            corner_radius: 0.0,
            x: 10.0,
            y: 20.0,
        })
        .unwrap();
    let id = workspace.document.selected.unwrap();
    workspace.begin_interaction();
    for x in 11..=80 {
        workspace
            .preview(Command::SetTransform {
                id,
                transform: Transform {
                    x: x as f32,
                    y: 20.0,
                    ..Default::default()
                },
            })
            .unwrap();
    }
    assert!(workspace.commit_interaction().unwrap());
    assert_eq!(workspace.document.layer(id).unwrap().transform.x, 80.0);
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(workspace.document.layer(id).unwrap().transform.x, 10.0);
}

#[test]
fn durable_project_preserves_alternate_futures() {
    let directory = test_directory("durable-branches");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("branches.prism");
    let session = spectrum_revisions::SessionId::new();
    let (mut project, mut document) = DurableProject::create(
        &project_path,
        &Document::new("Branches", 400, 300),
        test_actor("person:branches", spectrum_revisions::ActorKind::Human),
        session,
    )
    .unwrap();

    let first_command = Command::AddRectangle {
        name: Some("First".into()),
        width: 100,
        height: 80,
        color: [255, 0, 0, 255],
        corner_radius: 8.0,
        x: 20.0,
        y: 30.0,
    };
    apply_command(&mut document, first_command.clone()).unwrap();
    let first = project
        .commit(&[first_command], &document, "Added first rectangle")
        .unwrap();

    let second_command = Command::AddRectangle {
        name: Some("Second".into()),
        width: 60,
        height: 60,
        color: [0, 255, 0, 255],
        corner_radius: 0.0,
        x: 160.0,
        y: 30.0,
    };
    apply_command(&mut document, second_command.clone()).unwrap();
    let second = project
        .commit(&[second_command], &document, "Added second rectangle")
        .unwrap();
    document = project.undo().unwrap();
    assert_eq!(project.cursor(), first);
    assert_eq!(document.layers.len(), 1);

    let alternate_command = Command::AddText {
        text: "Alternate".into(),
        name: None,
        font_size: 32.0,
        color: [255, 255, 255, 255],
        x: 40.0,
        y: 160.0,
    };
    apply_command(&mut document, alternate_command.clone()).unwrap();
    let alternate = project
        .commit(&[alternate_command], &document, "Added alternate text")
        .unwrap();
    assert_ne!(second, alternate);

    let original_future = project.move_to(second).unwrap();
    assert_eq!(original_future.layers.len(), 2);
    assert!(matches!(
        original_future.layers[1].kind,
        LayerKind::Rectangle { .. }
    ));
    let alternate_future = project.move_to(alternate).unwrap();
    assert_eq!(alternate_future.layers.len(), 2);
    assert!(matches!(
        alternate_future.layers[1].kind,
        LayerKind::Text { .. }
    ));
    project.checkpoint().unwrap();
    drop(project);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_project_uses_sparse_compressed_snapshots() {
    let directory = test_directory("sparse-snapshots");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("compact.prism");
    let session = spectrum_revisions::SessionId::new();
    let mut document = Document::new("Compact", 400, 300);
    apply_command(
        &mut document,
        Command::AddRectangle {
            name: Some("Card".into()),
            width: 100,
            height: 80,
            color: [255, 0, 0, 255],
            corner_radius: 8.0,
            x: 0.0,
            y: 0.0,
        },
    )
    .unwrap();
    let (mut project, mut document) = DurableProject::create(
        &project_path,
        &document,
        test_actor("person:compact", spectrum_revisions::ActorKind::Human),
        session,
    )
    .unwrap();

    for x in 1..=250 {
        let command = Command::SetTransform {
            id: 1,
            transform: Transform {
                x: x as f32,
                ..document.layers[0].transform
            },
        };
        apply_command(&mut document, command.clone()).unwrap();
        project.commit(&[command], &document, "Moved card").unwrap();
    }
    let latest = project.cursor();
    project.checkpoint().unwrap();
    drop(project);

    let connection = rusqlite::Connection::open(&project_path).unwrap();
    let revision_count: u32 = connection
        .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
        .unwrap();
    let snapshot_count: u32 = connection
        .query_row("SELECT count(*) FROM snapshots", [], |row| row.get(0))
        .unwrap();
    let snapshot_bytes: i64 = connection
        .query_row("SELECT sum(length(bytes)) FROM snapshots", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(revision_count, 251);
    assert_eq!(snapshot_count, 3);
    assert!(snapshot_bytes < 16 * 1024);
    let legacy_snapshot_count: u32 = connection
        .query_row(
            "SELECT count(*) FROM snapshots WHERE version = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let compressed_snapshot_count: u32 = connection
        .query_row(
            "SELECT count(*) FROM snapshots WHERE version = 2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy_snapshot_count, 1);
    assert_eq!(compressed_snapshot_count, 2);
    drop(connection);
    let project_bytes = std::fs::metadata(&project_path).unwrap().len();
    eprintln!("250-action compact Prism project: {project_bytes} bytes");
    assert!(project_bytes < 512 * 1024);

    struct LegacyCompatibility;
    impl spectrum_revisions::Compatibility for LegacyCompatibility {
        fn supports_snapshot(&self, encoding: &spectrum_revisions::Encoding) -> bool {
            encoding.family == "spectrum.prism.document"
                && encoding.version == 1
                && encoding.required_capabilities.is_empty()
        }

        fn supports_operations(&self, encoding: &spectrum_revisions::Encoding) -> bool {
            encoding.family == "spectrum.prism.commands"
                && encoding.version == 1
                && encoding.required_capabilities.is_empty()
        }
    }
    let store = spectrum_revisions::RevisionStore::open(&project_path).unwrap();
    let legacy_plan = store.replay_plan(latest, &LegacyCompatibility).unwrap();
    assert_eq!(
        legacy_plan.snapshot_revision,
        store.project_info().unwrap().root_revision
    );
    assert_eq!(legacy_plan.steps.len(), 250);
    drop(store);

    let (reopened, document) = DurableProject::open(
        &project_path,
        test_actor("person:reopen", spectrum_revisions::ActorKind::Human),
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    assert_eq!(document.layers[0].transform.x, 250.0);
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_project_embeds_raster_assets_in_the_single_file() {
    let directory = test_directory("durable-assets");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    RgbaImage::from_pixel(8, 8, Rgba([12, 34, 56, 255]))
        .save(&source)
        .unwrap();
    let mut workspace = Workspace::new(Document::new("Portable", 16, 16), None);
    workspace
        .execute(Command::AddRaster {
            path: source.clone(),
            name: Some("Embedded".into()),
            x: 4.0,
            y: 4.0,
        })
        .unwrap();
    let project_path = directory.join("portable.prism");
    let session = spectrum_revisions::SessionId::new();
    let (project, materialized) = DurableProject::create(
        &project_path,
        &workspace.document,
        test_actor("person:assets", spectrum_revisions::ActorKind::Human),
        session,
    )
    .unwrap();
    assert_eq!(render_document(&materialized, None).unwrap().width(), 16);
    project.checkpoint().unwrap();
    drop(project);
    std::fs::remove_file(&source).unwrap();

    let (reopened, document) = DurableProject::open(
        &project_path,
        test_actor("agent:assets", spectrum_revisions::ActorKind::Agent),
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    assert_eq!(render_document(&document, None).unwrap().width(), 16);
    reopened.checkpoint().unwrap();
    drop(reopened);
    assert!(project_path.is_file());
    assert!(std::fs::metadata(&project_path).unwrap().len() > 8 * 8 * 4);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_workspace_auto_commits_and_restores_its_session_cursor() {
    let directory = test_directory("durable-workspace");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("workspace.prism");
    let session = spectrum_revisions::SessionId::new();
    let actor = test_actor("person:workspace", spectrum_revisions::ActorKind::Human);
    let mut workspace = Workspace::create_durable(
        Document::new("Workspace", 400, 300),
        &project_path,
        actor.clone(),
        session,
    )
    .unwrap();
    workspace
        .execute_batch(vec![
            Command::AddRectangle {
                name: Some("Card".into()),
                width: 100,
                height: 80,
                color: [255, 0, 0, 255],
                corner_radius: 8.0,
                x: 20.0,
                y: 30.0,
            },
            Command::SetOpacity {
                id: 1,
                opacity: 0.5,
            },
        ])
        .unwrap();
    assert!(!workspace.is_dirty());
    assert!(!sidecar_path(&project_path, "-wal").exists());
    assert!(!sidecar_path(&project_path, "-shm").exists());
    drop(workspace);

    let mut reopened = Workspace::open_as(&project_path, actor, session).unwrap();
    assert_eq!(reopened.document.layers.len(), 1);
    assert_eq!(reopened.document.layers[0].opacity, 0.5);
    reopened.execute(Command::Undo).unwrap();
    assert!(reopened.document.layers.is_empty());
    reopened.execute(Command::Redo).unwrap();
    assert_eq!(reopened.document.layers.len(), 1);
    reopened.save(None).unwrap();
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_history_navigation_is_idempotent_and_visibly_forks() {
    let directory = test_directory("workspace-history-tree");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("history.prism");
    let session = spectrum_revisions::SessionId::new();
    let mut workspace = Workspace::create_durable(
        Document::new("History", 400, 300),
        &project_path,
        test_actor("person:history", spectrum_revisions::ActorKind::Human),
        session,
    )
    .unwrap();
    workspace
        .execute(Command::AddRectangle {
            name: Some("First path".into()),
            width: 100,
            height: 80,
            color: [255, 0, 0, 255],
            corner_radius: 8.0,
            x: 20.0,
            y: 30.0,
        })
        .unwrap();
    let first = workspace.history().unwrap().unwrap().current;
    workspace
        .execute(Command::AddText {
            text: "Original future".into(),
            name: None,
            font_size: 24.0,
            color: [255, 255, 255, 255],
            x: 40.0,
            y: 150.0,
        })
        .unwrap();
    let original = workspace.history().unwrap().unwrap().current;
    let bytes_before = std::fs::read(&project_path).unwrap();
    assert!(!workspace.move_to_revision(original).unwrap());
    assert_eq!(std::fs::read(&project_path).unwrap(), bytes_before);

    assert!(workspace.move_to_revision(first).unwrap());
    assert_eq!(workspace.document.layers.len(), 1);
    workspace
        .execute(Command::AddText {
            text: "Alternate future".into(),
            name: None,
            font_size: 24.0,
            color: [255, 255, 255, 255],
            x: 40.0,
            y: 200.0,
        })
        .unwrap();
    let history = workspace.history().unwrap().unwrap();
    assert_eq!(history.revisions.len(), 4);
    assert_eq!(
        history
            .revisions
            .iter()
            .filter(|revision| revision.parent_id == Some(first))
            .count(),
        2
    );
    assert!(
        history
            .revisions
            .iter()
            .any(|revision| revision.id == original)
    );
    assert_eq!(
        history
            .sessions
            .iter()
            .find(|candidate| candidate.id == session)
            .unwrap()
            .cursor,
        history.current
    );
    drop(workspace);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn together_agent_session_live_follows_then_splits_on_human_edit() {
    let directory = test_directory("workspace-agent-together");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("agent-together.prism");
    let human_session = spectrum_revisions::SessionId::new();
    let mut human = Workspace::create_durable(
        Document::new("Together", 400, 300),
        &project_path,
        test_actor("person:together", spectrum_revisions::ActorKind::Human),
        human_session,
    )
    .unwrap();
    let collaboration = Workspace::start_collaboration(
        &project_path,
        Some(human_session),
        test_actor("agent:together", spectrum_revisions::ActorKind::Agent),
        spectrum_revisions::CollaborationMode::Together,
    )
    .unwrap();
    let mut agent = Workspace::open_session(&project_path, collaboration.agent_session).unwrap();
    agent
        .execute(Command::AddText {
            text: "Agent result".into(),
            name: None,
            font_size: 32.0,
            color: [255, 255, 255, 255],
            x: 20.0,
            y: 20.0,
        })
        .unwrap();
    assert!(matches!(
        human.sync_together().unwrap(),
        spectrum_revisions::CollaborationSync::Advanced { .. }
    ));
    assert_eq!(human.document.layers[0].name, "Agent result");

    human
        .execute(Command::AddRectangle {
            name: Some("Human direction".into()),
            width: 100,
            height: 80,
            color: [0, 255, 0, 255],
            corner_radius: 8.0,
            x: 100.0,
            y: 100.0,
        })
        .unwrap();
    agent
        .execute(Command::AddText {
            text: "Agent alternate".into(),
            name: None,
            font_size: 24.0,
            color: [255, 255, 255, 255],
            x: 20.0,
            y: 80.0,
        })
        .unwrap();
    assert!(matches!(
        human.sync_together().unwrap(),
        spectrum_revisions::CollaborationSync::Split(_)
    ));
    assert!(
        human
            .document
            .layers
            .iter()
            .any(|layer| layer.name == "Human direction")
    );
    assert!(
        human
            .document
            .layers
            .iter()
            .all(|layer| layer.name != "Agent alternate")
    );
    drop(agent);
    drop(human);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn read_only_load_does_not_create_a_session_or_grow_history() {
    let directory = test_directory("read-only-load");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("read-only.prism");
    let workspace = Workspace::create_durable(
        Document::new("Read only", 400, 300),
        &project_path,
        test_actor("person:reader", spectrum_revisions::ActorKind::Human),
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    drop(workspace);

    let before = std::fs::read(&project_path).unwrap();
    let connection = rusqlite::Connection::open(&project_path).unwrap();
    let sessions_before: u32 = connection
        .query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    drop(connection);

    let document = Workspace::load_read_only(&project_path).unwrap();
    assert_eq!(document.name, "Read only");

    let connection = rusqlite::Connection::open(&project_path).unwrap();
    let sessions_after: u32 = connection
        .query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    drop(connection);
    assert_eq!(sessions_after, sessions_before);
    assert_eq!(std::fs::read(&project_path).unwrap(), before);
    assert!(!sidecar_path(&project_path, "-wal").exists());
    assert!(!sidecar_path(&project_path, "-shm").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn reopening_the_same_session_does_not_change_the_project() {
    let directory = test_directory("stable-session-reopen");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("stable.prism");
    let session = spectrum_revisions::SessionId::new();
    let actor = test_actor("person:stable", spectrum_revisions::ActorKind::Human);
    let workspace = Workspace::create_durable(
        Document::new("Stable", 400, 300),
        &project_path,
        actor.clone(),
        session,
    )
    .unwrap();
    drop(workspace);
    let before = std::fs::read(&project_path).unwrap();

    let reopened = Workspace::open_as(&project_path, actor, session).unwrap();
    assert_eq!(reopened.document.name, "Stable");
    drop(reopened);
    assert_eq!(std::fs::read(&project_path).unwrap(), before);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_workspace_moves_without_losing_history_or_session_position() {
    let directory = test_directory("durable-move");
    let source_directory = directory.join("managed");
    let destination_directory = directory.join("chosen");
    std::fs::create_dir_all(&source_directory).unwrap();
    let source = source_directory.join("Moving.prism");
    let destination = destination_directory.join("Moving.prism");
    let session = spectrum_revisions::SessionId::new();
    let actor = test_actor("person:moving", spectrum_revisions::ActorKind::Human);
    let mut workspace = Workspace::create_durable(
        Document::new("Moving", 400, 300),
        &source,
        actor.clone(),
        session,
    )
    .unwrap();
    workspace
        .execute(Command::AddText {
            text: "Still here".into(),
            name: None,
            font_size: 32.0,
            color: [255, 255, 255, 255],
            x: 20.0,
            y: 30.0,
        })
        .unwrap();

    assert_eq!(workspace.move_project(&destination).unwrap(), destination);
    assert!(!source.exists());
    assert!(destination.is_file());
    assert!(!sidecar_path(&destination, "-wal").exists());
    assert!(!sidecar_path(&destination, "-shm").exists());
    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.layers.is_empty());
    workspace.execute(Command::Redo).unwrap();
    assert_eq!(workspace.document.layers.len(), 1);
    drop(workspace);

    let reopened = Workspace::open_as(&destination, actor, session).unwrap();
    assert_eq!(reopened.document.layers.len(), 1);
    assert_eq!(reopened.document.layers[0].name, "Still here");
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn canceled_interaction_restores_the_document() {
    let mut workspace = Workspace::new(Document::new("Gesture", 400, 300), None);
    workspace
        .execute(Command::AddText {
            text: "Prism".into(),
            name: None,
            font_size: 32.0,
            color: [255, 255, 255, 255],
            x: 4.0,
            y: 8.0,
        })
        .unwrap();
    let before = workspace.document.clone();
    let id = workspace.document.selected.unwrap();
    workspace.begin_interaction();
    workspace
        .preview(Command::SetOpacity { id, opacity: 0.25 })
        .unwrap();
    assert!(workspace.cancel_interaction());
    assert_eq!(workspace.document, before);
}

#[test]
fn saving_copies_external_sources_into_portable_assets() {
    let root = test_directory("portable");
    let source_directory = root.join("external");
    let project_directory = root.join("project");
    fs::create_dir_all(&source_directory).unwrap();
    fs::create_dir_all(&project_directory).unwrap();
    let source = source_directory.join("source.png");
    RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 255]))
        .save(&source)
        .unwrap();
    let project = project_directory.join("portable.prism");
    let mut workspace = Workspace::new(Document::new("Portable", 2, 2), Some(project.clone()));
    workspace
        .execute(Command::AddRaster {
            path: source.clone(),
            name: None,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    workspace.save(None).unwrap();
    let LayerKind::Raster {
        path,
        original_path,
    } = &workspace.document.layers[0].kind
    else {
        panic!("expected raster layer");
    };
    assert!(path.starts_with(fs::canonicalize(&project_directory).unwrap()));
    assert!(path.exists());
    assert_eq!(
        original_path.as_ref(),
        Some(&fs::canonicalize(&source).unwrap())
    );
    assert!(export_document(&workspace.document, &source, 90).is_err());
    let serialized = fs::read_to_string(project).unwrap();
    assert!(serialized.contains("portable-assets"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_mica_projects_remain_readable_and_writable() {
    let root = test_directory("legacy-mica");
    fs::create_dir_all(&root).unwrap();
    let project = root.join("legacy.mica");
    let document = Document::new("Legacy", 320, 240);
    save_document(&document, &project).unwrap();
    let loaded = load_document(&project).unwrap();
    assert_eq!(loaded.name, "Legacy");
    assert_eq!((loaded.width, loaded.height), (320, 240));
    fs::remove_dir_all(root).unwrap();
}
