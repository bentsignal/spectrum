use std::time::{SystemTime, UNIX_EPOCH};

use image::Rgba;

use crate::*;

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-selection-{label}-{stamp}"))
}

fn test_actor() -> spectrum_revisions::Actor {
    spectrum_revisions::Actor {
        id: "person:selection-test".into(),
        display_name: "Selection test".into(),
        kind: spectrum_revisions::ActorKind::Human,
    }
}

#[test]
fn legacy_documents_default_to_no_selection_and_migrate_to_v4() {
    let mut value = serde_json::to_value(Document::new("Legacy", 80, 60)).unwrap();
    value["version"] = serde_json::json!(3);
    value.as_object_mut().unwrap().remove("selection");
    let mut document: Document = serde_json::from_value(value).unwrap();
    assert_eq!(document.selection, None);
    document.migrate().unwrap();
    assert_eq!(document.version, PRISM_VERSION);
    assert_eq!(document.selection, None);
}

#[test]
fn selection_validation_clips_to_canvas_and_rejects_empty_or_disjoint_rectangles() {
    let mut workspace = Workspace::new(Document::new("Selection", 100, 80), None);
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(90, 70, 40, 30)),
        })
        .unwrap();
    assert_eq!(
        workspace.document.selection,
        Some(Selection::rectangle(90, 70, 10, 10))
    );
    let before = workspace.document.clone();
    assert!(
        workspace
            .execute(Command::SetSelection {
                selection: Some(Selection::rectangle(10, 10, 0, 4)),
            })
            .is_err()
    );
    assert_eq!(workspace.document, before);
    assert!(
        workspace
            .execute(Command::SetSelection {
                selection: Some(Selection::rectangle(100, 20, 4, 4)),
            })
            .is_err()
    );
}

#[test]
fn crop_intersects_and_translates_persistent_selection() {
    let mut workspace = Workspace::new(Document::new("Crop", 100, 80), None);
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(30, 20, 50, 40)),
        })
        .unwrap();
    workspace
        .execute(Command::CropCanvas {
            x: 50,
            y: 10,
            width: 40,
            height: 50,
        })
        .unwrap();
    assert_eq!(
        workspace.document.selection,
        Some(Selection::rectangle(0, 10, 30, 40))
    );
}

#[test]
fn crop_to_selection_uses_exact_bounds_offsets_content_and_clears_selection() {
    let mut document = Document::new("Crop to selection", 100, 80);
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            x: 35.0,
            y: 24.0,
            ..Default::default()
        },
        kind: LayerKind::Rectangle {
            width: 20,
            height: 10,
            color: [10, 20, 30, 255],
            corner_radius: 0.0,
        },
        ..Default::default()
    });
    document.next_id = 2;
    document.guides = vec![
        Guide {
            id: 1,
            orientation: GuideOrientation::Vertical,
            position: 45.0,
        },
        Guide {
            id: 2,
            orientation: GuideOrientation::Horizontal,
            position: 5.0,
        },
    ];
    document.next_guide_id = 3;
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(20, 10, 50, 40)),
        })
        .unwrap();

    let output = workspace.execute(Command::CropToSelection).unwrap();

    assert_eq!(output.action, "crop_to_selection");
    assert_eq!(
        (workspace.document.width, workspace.document.height),
        (50, 40)
    );
    assert_eq!(workspace.document.selection, None);
    assert_eq!(
        (
            workspace.document.layers[0].transform.x,
            workspace.document.layers[0].transform.y,
        ),
        (15.0, 14.0)
    );
    assert_eq!(workspace.document.guides.len(), 1);
    assert_eq!(workspace.document.guide(1).unwrap().position, 25.0);
}

#[test]
fn crop_to_selection_rejects_missing_invalid_and_full_canvas_selections_atomically() {
    let mut workspace = Workspace::new(Document::new("Crop errors", 100, 80), None);
    let before = workspace.document.clone();
    assert!(workspace.execute(Command::CropToSelection).is_err());
    assert_eq!(workspace.document, before);

    workspace.document.selection = Some(Selection::rectangle(100, 10, 20, 20));
    let before = workspace.document.clone();
    assert!(workspace.execute(Command::CropToSelection).is_err());
    assert_eq!(workspace.document, before);

    workspace.document.selection = Some(Selection::rectangle(0, 0, 100, 80));
    let before = workspace.document.clone();
    assert!(workspace.execute(Command::CropToSelection).is_err());
    assert_eq!(workspace.document, before);
}

#[test]
fn fill_creates_one_editable_layer_and_undo_preserves_original_content() {
    let mut document = Document::new("Fill", 40, 30);
    document.layers.push(Layer {
        id: 1,
        name: "Original".into(),
        transform: Transform {
            x: 8.0,
            y: 6.0,
            rotation: 31.0,
            ..Default::default()
        },
        kind: LayerKind::Rectangle {
            width: 18,
            height: 12,
            color: [20, 40, 80, 255],
            corner_radius: 2.0,
        },
        ..Default::default()
    });
    document.next_id = 2;
    let original = document.layers[0].clone();
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(3, 4, 11, 7)),
        })
        .unwrap();
    workspace
        .execute(Command::FillSelection {
            color: [200, 30, 90, 220],
            name: Some("Wash".into()),
        })
        .unwrap();
    assert_eq!(workspace.document.layers[0], original);
    assert_eq!(workspace.document.layers.len(), 2);
    let fill = &workspace.document.layers[1];
    assert_eq!(fill.name, "Wash");
    assert_eq!((fill.transform.x, fill.transform.y), (3.0, 4.0));
    assert_eq!(
        fill.kind,
        LayerKind::Rectangle {
            width: 11,
            height: 7,
            color: [200, 30, 90, 220],
            corner_radius: 0.0,
        }
    );
    assert_eq!(
        workspace.document.selection,
        Some(Selection::rectangle(3, 4, 11, 7))
    );
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(workspace.document.layers, vec![original]);
    assert_eq!(
        workspace.document.selection,
        Some(Selection::rectangle(3, 4, 11, 7))
    );
}

#[test]
fn fill_never_rewrites_or_repoints_original_raster_bytes() {
    let directory = test_directory("immutable-raster");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("original.png");
    image::RgbaImage::from_pixel(8, 6, Rgba([7, 18, 29, 255]))
        .save(&source)
        .unwrap();
    let before_bytes = std::fs::read(&source).unwrap();
    let canonical = std::fs::canonicalize(&source).unwrap();
    let mut workspace = Workspace::new(Document::new("Immutable", 20, 16), None);
    workspace
        .execute(Command::AddRaster {
            path: source,
            name: None,
            x: 2.0,
            y: 3.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(1, 1, 5, 4)),
        })
        .unwrap();
    workspace
        .execute(Command::FillSelection {
            color: [90, 80, 70, 200],
            name: None,
        })
        .unwrap();
    let LayerKind::Raster {
        path,
        original_path,
    } = &workspace.document.layers[0].kind
    else {
        panic!("original layer should remain raster");
    };
    assert_eq!(path, &canonical);
    assert_eq!(original_path.as_ref(), Some(&canonical));
    assert_eq!(std::fs::read(&canonical).unwrap(), before_bytes);
    drop(workspace);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn fill_pixels_match_export_over_rotated_content() {
    let directory = test_directory("export");
    std::fs::create_dir_all(&directory).unwrap();
    let export = directory.join("selection.png");
    let mut document = Document::new("Pixels", 16, 12);
    document.background = [0, 0, 0, 0];
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            x: 7.0,
            y: 3.0,
            rotation: 23.0,
            ..Default::default()
        },
        kind: LayerKind::Rectangle {
            width: 6,
            height: 5,
            color: [20, 30, 40, 255],
            corner_radius: 0.0,
        },
        ..Default::default()
    });
    document.next_id = 2;
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(2, 3, 4, 3)),
        })
        .unwrap();
    workspace
        .execute(Command::FillSelection {
            color: [240, 60, 80, 255],
            name: None,
        })
        .unwrap();
    let rendered = render_document(&workspace.document, None)
        .unwrap()
        .to_rgba8();
    for y in 3..6 {
        for x in 2..6 {
            assert_eq!(*rendered.get_pixel(x, y), Rgba([240, 60, 80, 255]));
        }
    }
    export_document(&workspace.document, &export, 92).unwrap();
    assert_eq!(image::open(&export).unwrap().to_rgba8(), rendered);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_selection_and_fill_each_commit_exactly_one_revision() {
    let directory = test_directory("durable");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("fill.prism");
    let session = spectrum_revisions::SessionId::new();
    let mut workspace = Workspace::create_durable(
        Document::new("Durable fill", 80, 60),
        &path,
        test_actor(),
        session,
    )
    .unwrap();
    assert_eq!(workspace.history().unwrap().unwrap().revisions.len(), 1);
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(8, 9, 20, 15)),
        })
        .unwrap();
    assert_eq!(workspace.history().unwrap().unwrap().revisions.len(), 2);
    workspace
        .execute(Command::FillSelection {
            color: [11, 22, 33, 255],
            name: None,
        })
        .unwrap();
    assert_eq!(workspace.history().unwrap().unwrap().revisions.len(), 3);
    drop(workspace);

    let reopened = Workspace::open_as(&path, test_actor(), session).unwrap();
    assert_eq!(
        reopened.document.selection,
        Some(Selection::rectangle(8, 9, 20, 15))
    );
    assert_eq!(reopened.document.layers.len(), 1);
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_crop_to_selection_is_one_revision_with_reopen_undo_and_redo_parity() {
    let directory = test_directory("durable-crop");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("crop.prism");
    let session = spectrum_revisions::SessionId::new();
    let mut document = Document::new("Durable crop", 120, 90);
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            x: 40.0,
            y: 30.0,
            ..Default::default()
        },
        kind: LayerKind::Rectangle {
            width: 24,
            height: 18,
            color: [40, 80, 120, 255],
            corner_radius: 2.0,
        },
        ..Default::default()
    });
    document.next_id = 2;
    document.guides.push(Guide {
        id: 1,
        orientation: GuideOrientation::Horizontal,
        position: 35.0,
    });
    document.next_guide_id = 2;
    let mut workspace = Workspace::create_durable(document, &path, test_actor(), session).unwrap();
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(20, 15, 60, 45)),
        })
        .unwrap();
    let before_crop = workspace.document.clone();
    let revisions_before = workspace.history().unwrap().unwrap().revisions.len();

    workspace.execute(Command::CropToSelection).unwrap();

    assert_eq!(
        workspace.history().unwrap().unwrap().revisions.len(),
        revisions_before + 1
    );
    let cropped = workspace.document.clone();
    assert_eq!((cropped.width, cropped.height), (60, 45));
    assert_eq!(cropped.selection, None);
    assert_eq!(
        (cropped.layers[0].transform.x, cropped.layers[0].transform.y),
        (20.0, 15.0)
    );
    assert_eq!(cropped.guide(1).unwrap().position, 20.0);
    drop(workspace);

    let connection = rusqlite::Connection::open(&path).unwrap();
    let (operation_version, operation_bytes): (u32, Vec<u8>) = connection
        .query_row(
            "SELECT version, bytes FROM operation_payloads WHERE instr(CAST(bytes AS TEXT), 'crop_to_selection') > 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(operation_version, PRISM_COMMAND_OPERATIONS_VERSION);
    assert_eq!(
        serde_json::from_slice::<Vec<Command>>(&operation_bytes).unwrap(),
        vec![Command::CropToSelection]
    );
    drop(connection);

    let mut reopened = Workspace::open_as(&path, test_actor(), session).unwrap();
    assert_eq!(reopened.document, cropped);
    reopened.execute(Command::Undo).unwrap();
    assert_eq!(reopened.document, before_crop);
    reopened.execute(Command::Redo).unwrap();
    assert_eq!(reopened.document, cropped);
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}
