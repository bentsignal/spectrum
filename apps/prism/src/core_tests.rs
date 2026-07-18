use super::*;
use image::{Rgba, RgbaImage};
use std::time::{SystemTime, UNIX_EPOCH};

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-{label}-{stamp}"))
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
    assert!(workspace.commit_interaction());
    assert_eq!(workspace.document.layer(id).unwrap().transform.x, 80.0);
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(workspace.document.layer(id).unwrap().transform.x, 10.0);
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
