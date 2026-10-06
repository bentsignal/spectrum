use super::*;
use image::{Rgba, RgbaImage};
use std::{
    ffi::OsString,
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("canvas-{label}-{stamp}"))
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
    let mut workspace = Workspace::new(Document::default());
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
    let mut workspace = Workspace::new(document.clone());
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
    let mut workspace = Workspace::new(Document::new("Poster", 400, 200));
    workspace
        .execute(Command::AddText {
            text: "Prism".into(),
            name: None,
            font_size: 48.0,
            color: [255, 255, 255, 255],
            x: 30.0,
            y: 40.0,
            shaping: Default::default(),
        })
        .unwrap();
    let rendered = render_document(&workspace.document, None).unwrap();
    assert_eq!((rendered.width(), rendered.height()), (400, 200));
}

#[test]
fn automatic_text_names_follow_content_without_overwriting_manual_names() {
    let mut workspace = Workspace::new(Document::new("Names", 400, 200));
    workspace
        .execute(Command::AddText {
            text: "Title\n".into(),
            name: None,
            font_size: 48.0,
            color: [255, 255, 255, 255],
            x: 30.0,
            y: 40.0,
            shaping: Default::default(),
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
    let mut workspace = Workspace::new(Document::new("Rotate", 100, 100));
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
    let mut workspace = Workspace::new(Document::new("Safety", 4, 4));
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
    let mut workspace = Workspace::new(Document::new("Finite", 20, 20));
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
    let mut workspace = Workspace::new(Document::new("Selection", 20, 20));
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
fn durable_workspace_auto_commits_and_restores_its_session_cursor() {
    let directory = test_directory("durable-workspace");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("workspace.spectrum");
    let session = spectrum_revisions::SessionId::new();
    let actor = test_actor("person:workspace", spectrum_revisions::ActorKind::Human);
    let mut workspace = Workspace::create(
        &project_path,
        Document::new("Workspace", 400, 300),
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
    assert!(!sidecar_path(&project_path, "-wal").exists());
    assert!(!sidecar_path(&project_path, "-shm").exists());
    drop(workspace);

    let mut reopened = Workspace::open(&project_path, actor, session).unwrap();
    assert_eq!(reopened.document.layers.len(), 1);
    assert_eq!(reopened.document.layers[0].opacity, 0.5);
    reopened.execute(Command::Undo).unwrap();
    assert!(reopened.document.layers.is_empty());
    reopened.execute(Command::Redo).unwrap();
    assert_eq!(reopened.document.layers.len(), 1);
    reopened.checkpoint().unwrap();
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_history_navigation_is_idempotent_and_visibly_forks() {
    let directory = test_directory("workspace-history-tree");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("history.spectrum");
    let session = spectrum_revisions::SessionId::new();
    let mut workspace = Workspace::create(
        &project_path,
        Document::new("History", 400, 300),
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
            shaping: Default::default(),
        })
        .unwrap();
    let original = workspace.history().unwrap().unwrap().current;
    let bytes_before = std::fs::read(&project_path).unwrap();
    assert!(!workspace.move_to(original).unwrap());
    assert_eq!(std::fs::read(&project_path).unwrap(), bytes_before);

    assert!(workspace.move_to(first).unwrap());
    assert_eq!(workspace.document.layers.len(), 1);
    workspace
        .execute(Command::AddText {
            text: "Alternate future".into(),
            name: None,
            font_size: 24.0,
            color: [255, 255, 255, 255],
            x: 40.0,
            y: 200.0,
            shaping: Default::default(),
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
    let project_path = directory.join("agent-together.spectrum");
    let human_session = spectrum_revisions::SessionId::new();
    let mut human = Workspace::create(
        &project_path,
        Document::new("Together", 400, 300),
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
            shaping: Default::default(),
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
            shaping: Default::default(),
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
    let project_path = directory.join("read-only.spectrum");
    let workspace = Workspace::create(
        &project_path,
        Document::new("Read only", 400, 300),
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

    let document = Workspace::read(&project_path).unwrap();
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
    let project_path = directory.join("stable.spectrum");
    let session = spectrum_revisions::SessionId::new();
    let actor = test_actor("person:stable", spectrum_revisions::ActorKind::Human);
    let workspace = Workspace::create(
        &project_path,
        Document::new("Stable", 400, 300),
        actor.clone(),
        session,
    )
    .unwrap();
    drop(workspace);
    let before = std::fs::read(&project_path).unwrap();

    let reopened = Workspace::open(&project_path, actor, session).unwrap();
    assert_eq!(reopened.document.name, "Stable");
    drop(reopened);
    assert_eq!(std::fs::read(&project_path).unwrap(), before);
    std::fs::remove_dir_all(directory).unwrap();
}
