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

fn triangle_lasso() -> LassoPath {
    LassoPath::new(vec![
        LassoPoint::from_canvas(2.0, 2.0).unwrap(),
        LassoPoint::from_canvas(14.0, 2.0).unwrap(),
        LassoPoint::from_canvas(2.0, 14.0).unwrap(),
    ])
    .unwrap()
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
fn soft_disconnected_fill_preserves_alpha_in_export_and_region_preview() {
    let directory = test_directory("soft-fill-export");
    std::fs::create_dir_all(&directory).unwrap();
    let mut document = Document::new("Soft fill", 8, 4);
    document.background = [0, 0, 0, 0];
    let alpha = vec![255, 0, 128, 0, 0, 255, 0, 64];
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::color_mask(1, 1, 4, 2, alpha.clone())),
        })
        .unwrap();
    workspace
        .execute(Command::FillSelection {
            color: [200, 20, 40, 200],
            name: Some("Soft pixels".into()),
        })
        .unwrap();
    let mask = workspace.document.layers[0].pixel_mask.as_ref().unwrap();
    assert_eq!(mask.alpha.as_ref(), alpha);
    let full = render_document(&workspace.document, None)
        .unwrap()
        .to_rgba8();
    assert_eq!(full.get_pixel(1, 1)[3], 200);
    assert_eq!(full.get_pixel(2, 1)[3], 0);
    assert_eq!(full.get_pixel(3, 1)[3], 100);
    assert_eq!(full.get_pixel(4, 2)[3], 50);
    let region = render_document_region_scaled(
        &workspace.document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: 8,
            height: 4,
        },
    )
    .unwrap()
    .to_rgba8();
    assert_eq!(region, full);
    let export = directory.join("soft-fill.png");
    export_document(&workspace.document, &export, 92).unwrap();
    assert_eq!(image::open(&export).unwrap().to_rgba8(), full);

    workspace
        .execute(Command::SetTransform {
            id: 1,
            transform: Transform {
                x: 2.0,
                y: 0.0,
                scale_x: 1.25,
                scale_y: 1.25,
                rotation: 18.0,
            },
        })
        .unwrap();
    let transformed = render_document(&workspace.document, None)
        .unwrap()
        .to_rgba8();
    let transformed_region = render_document_region_scaled(
        &workspace.document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: 8,
            height: 4,
        },
    )
    .unwrap()
    .to_rgba8();
    assert_eq!(transformed_region, transformed);
    export_document(&workspace.document, &export, 92).unwrap();
    assert_eq!(image::open(&export).unwrap().to_rgba8(), transformed);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_magic_wand_is_one_v6_marker_plus_exact_v5_snapshot() {
    let directory = test_directory("durable-wand");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("wand.prism");
    let session = spectrum_revisions::SessionId::new();
    let mut document = Document::new("Durable wand", 10, 4);
    document.background = [0, 0, 0, 255];
    for (id, x) in [(1, 1.0), (2, 7.0)] {
        document.layers.push(Layer {
            id,
            transform: Transform {
                x,
                y: 1.0,
                ..Default::default()
            },
            kind: LayerKind::Rectangle {
                width: 2,
                height: 2,
                color: [220, 20, 30, 255],
                corner_radius: 0.0,
            },
            ..Default::default()
        });
    }
    document.next_id = 3;
    let mut workspace = Workspace::create_durable(document, &path, test_actor(), session).unwrap();
    let before = workspace.document.clone();
    let revisions_before = workspace.history().unwrap().unwrap().revisions.len();
    let command = Command::MagicWandSelection {
        x: 1,
        y: 1,
        tolerance: 0,
        contiguous: false,
        antialias: false,
        resolved_selection: None,
    };
    workspace.execute(command.clone()).unwrap();
    assert_eq!(
        workspace.history().unwrap().unwrap().revisions.len(),
        revisions_before + 1
    );
    let selected = workspace.document.selection.clone().unwrap();
    assert_eq!(selected.bounds(), (1, 1, 8, 2));
    assert!(selected.alpha().is_some());
    let size_after_first = std::fs::metadata(&path).unwrap().len();
    workspace.execute(command).unwrap();
    assert_eq!(
        workspace.history().unwrap().unwrap().revisions.len(),
        revisions_before + 1
    );
    assert_eq!(std::fs::metadata(&path).unwrap().len(), size_after_first);
    drop(workspace);

    let connection = rusqlite::Connection::open(&path).unwrap();
    let (version, bytes): (u32, Vec<u8>) = connection
        .query_row(
            "SELECT version, bytes FROM operation_payloads WHERE instr(CAST(bytes AS TEXT), 'magic_wand_snapshot') > 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(version, 6);
    assert!(
        bytes.len() < 512,
        "marker operation unexpectedly stored mask bytes"
    );
    let v5_snapshots: u32 = connection
        .query_row(
            "SELECT count(*) FROM snapshots WHERE version = 5",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(v5_snapshots, 1);
    drop(connection);

    let marker = Command::MagicWandSnapshot {
        x: 1,
        y: 1,
        tolerance: 0,
        contiguous: false,
        antialias: false,
    };
    let mut without_snapshot = before.clone();
    assert!(crate::apply_command(&mut without_snapshot, marker).is_err());
    assert_eq!(without_snapshot, before);

    let mut reopened = Workspace::open_as(&path, test_actor(), session).unwrap();
    // Reopen succeeds although the marker is fail-closed, proving the replay
    // plan starts from the same-revision v5 snapshot and has zero marker steps.
    assert_eq!(reopened.document.selection, Some(selected.clone()));
    reopened.execute(Command::Undo).unwrap();
    assert_eq!(reopened.document, before);
    reopened.execute(Command::Redo).unwrap();
    assert_eq!(reopened.document.selection, Some(selected));
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_lasso_is_one_v9_revision_and_reopens_exactly() {
    let directory = test_directory("durable-lasso");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("lasso.prism");
    let session = spectrum_revisions::SessionId::new();
    let document = Document::new("Durable lasso", 32, 32);
    let before = document.clone();
    let mut workspace = Workspace::create_durable(document, &path, test_actor(), session).unwrap();
    let revisions_before = workspace.history().unwrap().unwrap().revisions.len();
    let command = Command::LassoSelection {
        points: triangle_lasso(),
        mode: SelectionCombineMode::Replace,
        antialias: true,
    };
    workspace.execute(command.clone()).unwrap();
    let selected = workspace.document.selection.clone().unwrap();
    assert!(selected.alpha().is_some());
    assert_eq!(
        workspace.history().unwrap().unwrap().revisions.len(),
        revisions_before + 1
    );
    workspace.execute(command).unwrap();
    assert_eq!(
        workspace.history().unwrap().unwrap().revisions.len(),
        revisions_before + 1
    );
    drop(workspace);

    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: u32 = connection
        .query_row(
            "SELECT version FROM operation_payloads WHERE instr(CAST(bytes AS TEXT), 'lasso_selection') > 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, 9);
    drop(connection);

    let mut reopened = Workspace::open_as(&path, test_actor(), session).unwrap();
    assert_eq!(reopened.document.selection, Some(selected.clone()));
    reopened.execute(Command::Undo).unwrap();
    assert_eq!(reopened.document, before);
    reopened.execute(Command::Redo).unwrap();
    assert_eq!(reopened.document.selection, Some(selected));
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failed_lasso_is_atomic_for_document_and_durable_history() {
    let directory = test_directory("failed-lasso");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("failed.prism");
    let session = spectrum_revisions::SessionId::new();
    let document = Document::new("Failed lasso", 32, 32);
    let mut workspace = Workspace::create_durable(document, &path, test_actor(), session).unwrap();
    let before = workspace.document.clone();
    let revisions_before = workspace.history().unwrap().unwrap().revisions.len();
    let line = LassoPath::new(vec![
        LassoPoint::from_canvas(1.0, 1.0).unwrap(),
        LassoPoint::from_canvas(4.0, 4.0).unwrap(),
        LassoPoint::from_canvas(8.0, 8.0).unwrap(),
    ])
    .unwrap();
    assert!(
        workspace
            .execute(Command::LassoSelection {
                points: line,
                mode: SelectionCombineMode::Replace,
                antialias: true,
            })
            .is_err()
    );
    assert_eq!(workspace.document, before);
    assert_eq!(
        workspace.history().unwrap().unwrap().revisions.len(),
        revisions_before
    );
    drop(workspace);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn lasso_soft_selection_interoperates_with_fill_and_frozen_brush_clip() {
    let mut workspace = Workspace::new(Document::new("Lasso interop", 32, 32), None);
    workspace
        .execute(Command::LassoSelection {
            points: triangle_lasso(),
            mode: SelectionCombineMode::Replace,
            antialias: true,
        })
        .unwrap();
    let selection = workspace.document.selection.clone().unwrap();
    let selection_bounds = selection.bounds();
    let expected_alpha = selection.alpha().unwrap().to_vec();
    workspace
        .execute(Command::FillSelection {
            color: [200, 20, 40, 255],
            name: None,
        })
        .unwrap();
    assert_eq!(
        workspace.document.layers[0]
            .pixel_mask
            .as_ref()
            .unwrap()
            .alpha
            .as_ref(),
        expected_alpha.as_slice()
    );
    let stroke = BrushStroke::new(
        BrushStyle {
            mode: BrushMode::Paint,
            color: [255; 4],
            size: 6.0,
            hardness: 1.0,
            opacity: 1.0,
            spacing: 0.25,
        },
        vec![
            BrushSample {
                x: 6.0,
                y: 6.0,
                pressure: 1.0,
            },
            BrushSample {
                x: 10.0,
                y: 10.0,
                pressure: 1.0,
            },
        ],
    )
    .unwrap();
    workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: None,
            width: 32,
            height: 32,
            stroke,
            selection: PaintSelection::Current,
        })
        .unwrap();
    workspace
        .execute(Command::SetSelection { selection: None })
        .unwrap();
    let LayerKind::Paint { program } = &workspace.document.layers[1].kind else {
        panic!("expected Paint layer")
    };
    let Some(BrushClip::Alpha {
        x,
        y,
        width,
        height,
        alpha,
    }) = &program.strokes[0].clip
    else {
        panic!("expected frozen soft lasso clip")
    };
    assert_eq!((*x, *y, *width, *height), selection_bounds);
    assert_eq!(alpha.as_ref(), expected_alpha.as_slice());
}

#[test]
fn inline_mask_budget_failures_are_atomic_for_execute_and_preview() {
    let alpha = Arc::<[u8]>::from(vec![255; 4_096 * 4_096]);
    let mask = PixelMask::new(4_096, 4_096, alpha);
    let mut document = Document::new("Mask budget", 4_096, 4_096);
    for id in 1..=4 {
        document.layers.push(Layer {
            id,
            pixel_mask: Some(mask.clone()),
            kind: LayerKind::Rectangle {
                width: 4_096,
                height: 4_096,
                color: [20, 40, 60, 255],
                corner_radius: 0.0,
            },
            ..Default::default()
        });
    }
    document.next_id = 5;
    let mut workspace = Workspace::new(document, None);
    let before = workspace.document.clone();
    assert!(
        workspace
            .execute(Command::SetSelection {
                selection: Some(Selection::color_mask(0, 0, 1, 1, vec![128])),
            })
            .is_err()
    );
    assert_eq!(workspace.document, before);

    workspace.begin_interaction();
    assert!(
        workspace
            .preview(Command::DuplicateLayer { id: 1 })
            .is_err()
    );
    assert_eq!(workspace.document, before);
    workspace.cancel_interaction();
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
    assert_eq!(operation_version, 5);
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
