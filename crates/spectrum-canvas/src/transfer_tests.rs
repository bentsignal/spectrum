use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    BlendMode, Command, Document, DropShadow, GradientStop, Layer, LayerKind, LayerMask,
    LayerStyle, LayerTransfer, PixelMask, ShapeFill, ShapeGradient, ShapeStroke, TextAlignment,
    TextEffects, TextTypography, Transform, Workspace, render_document,
};

#[test]
fn gradient_transfer_rejects_forged_fields_and_unknown_versions() {
    let mut document = Document::new("Gradient transfer", 64, 64);
    document.layers.push(Layer {
        id: 1,
        shape_fill: Some(ShapeFill::Gradient(ShapeGradient {
            kind: crate::GradientKind::Radial,
            spread: crate::GradientSpread::Repeat,
            stops: vec![
                GradientStop::new(0.0, [255, 0, 0, 255]),
                GradientStop::new(0.5, [0, 255, 0, 128]),
                GradientStop::new(1.0, [0, 0, 255, 0]),
            ],
            ..Default::default()
        })),
        kind: LayerKind::Ellipse {
            width: 20,
            height: 20,
            color: [255; 4],
        },
        ..Default::default()
    });
    document.selected = Some(1);
    let transfer = LayerTransfer::from_selected(&document).unwrap();
    assert_eq!(transfer.version, crate::LAYER_TRANSFER_VERSION);
    let encoded = transfer.to_json().unwrap();
    let forgeries = [
        encoded.replacen(r#""kind":"radial""#, r#""kind":"radial","raduis":0.5"#, 1),
        encoded.replacen(r#""kind":"radial""#, r#""kind":"radial","kind":"angle""#, 1),
        encoded.replacen(r#""position":0.0"#, r#""position":0.0,"position":0.1"#, 1),
        encoded.replacen(
            r#""kind":"radial""#,
            r#""kind":"radial","interpolation":"linear_rgb_v1""#,
            1,
        ),
    ];
    for forged in forgeries {
        assert_ne!(forged, encoded);
        assert!(LayerTransfer::from_json(&forged).is_err());
    }

    let mut old = transfer;
    old.version = crate::LAYER_TRANSFER_VERSION + 1;
    assert!(old.validate_envelope_metadata().is_err());
}

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!("canvas-transfer-{label}-{stamp}"))
}

fn test_actor() -> spectrum_revisions::Actor {
    spectrum_revisions::Actor {
        id: "person:transfer-test".into(),
        display_name: "Transfer test".into(),
        kind: spectrum_revisions::ActorKind::Human,
    }
}

#[test]
fn transfer_preserves_every_layer_field_except_local_ids_in_one_undo_step() {
    let mut source = Document::new("Source", 800, 600);
    let mut layer = Layer {
        id: 41,
        name: "Exact card".into(),
        visible: false,
        locked: true,
        opacity: 0.42,
        blend_mode: BlendMode::SoftLight,
        transform: Transform {
            x: 123.0,
            y: -45.0,
            scale_x: 1.25,
            scale_y: 0.75,
            rotation: 317.0,
        },
        mask: LayerMask {
            enabled: true,
            x: 0.1,
            y: 0.2,
            width: 0.7,
            height: 0.6,
            invert: true,
        },
        style: LayerStyle {
            drop_shadow: Some(DropShadow::default()),
            ..LayerStyle::default()
        },
        shape_fill: Some(ShapeFill::Gradient(ShapeGradient {
            angle: 35.0,
            stops: vec![
                GradientStop::new(0.0, [8, 16, 32, 255]),
                GradientStop::new(1.0, [240, 180, 90, 220]),
            ],
            ..ShapeGradient::default()
        })),
        stroke: ShapeStroke {
            enabled: true,
            width: 7.0,
            color: [1, 2, 3, 4],
        },
        clip_to_below: true,
        kind: LayerKind::Rectangle {
            width: 321,
            height: 123,
            color: [10, 20, 30, 40],
            corner_radius: 19.0,
        },
        ..Default::default()
    };
    layer.adjustments.exposure = 0.75;
    layer.adjustments.contrast = 12.0;
    source.layers.push(layer.clone());
    source.selected = Some(layer.id);

    let transfer = LayerTransfer::from_selected(&source).unwrap();
    assert_eq!(transfer.layer.id, 0);
    let decoded = LayerTransfer::from_json(&transfer.to_json().unwrap()).unwrap();
    assert_eq!(decoded, transfer);

    let mut destination = Workspace::new(Document::new("Destination", 800, 600));
    destination
        .execute(Command::AddEllipse {
            name: Some("Below".into()),
            width: 80,
            height: 80,
            color: [255; 4],
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    let inserted = destination
        .execute(Command::InsertLayer {
            transfer: Box::new(transfer),
            index: None,
        })
        .unwrap()
        .layer_ids[0];
    let mut expected = layer;
    expected.id = inserted;
    assert_eq!(destination.document.layers[1], expected);
    assert_eq!(destination.document.selected, Some(inserted));

    destination.execute(Command::Undo).unwrap();
    assert_eq!(destination.document.layers.len(), 1);
    destination.execute(Command::Redo).unwrap();
    assert_eq!(destination.document.layers[1], expected);
    assert_eq!(destination.document.selected, Some(inserted));
}

#[test]
fn transfer_rejects_foreign_ids_versions_and_invalid_values_atomically() {
    let mut transfer = LayerTransfer::from_document(
        &Document {
            layers: vec![Layer {
                id: 9,
                kind: LayerKind::Ellipse {
                    width: 10,
                    height: 10,
                    color: [255; 4],
                },
                ..Default::default()
            }],
            ..Document::new("Source", 20, 20)
        },
        9,
    )
    .unwrap();
    let compatible_version = transfer.version;
    transfer.version = crate::LAYER_TRANSFER_VERSION + 1;
    let encoded = serde_json::to_string(&transfer).unwrap();
    assert!(LayerTransfer::from_json(&encoded).is_err());

    transfer.version = compatible_version;
    transfer.layer.id = 99;
    assert!(transfer.to_json().is_err());
    transfer.layer.id = 0;
    transfer.layer.opacity = f32::NAN;
    let before = Document::new("Destination", 20, 20);
    let mut workspace = Workspace::new(before.clone());
    assert!(
        workspace
            .execute(Command::InsertLayer {
                transfer: Box::new(transfer),
                index: None,
            })
            .is_err()
    );
    assert_eq!(workspace.document, before);
}

#[test]
fn pixel_mask_transfer_round_trips_pixels_and_rejects_tampering() {
    let mut source = Document::new("Masked source", 16, 12);
    source.background = [0; 4];
    source.layers.push(Layer {
        id: 7,
        name: "Masked shape".into(),
        transform: Transform {
            x: 3.0,
            y: 2.0,
            ..Default::default()
        },
        kind: LayerKind::Rectangle {
            width: 3,
            height: 2,
            color: [20, 80, 220, 255],
            corner_radius: 0.0,
        },
        pixel_mask: Some(PixelMask::new(3, 2, vec![255, 0, 128, 0, 255, 64])),
        ..Default::default()
    });
    source.selected = Some(7);
    let expected = render_document(&source, None).unwrap().to_rgba8();

    let transfer = LayerTransfer::from_selected(&source).unwrap();
    let decoded = LayerTransfer::from_json(&transfer.to_json().unwrap()).unwrap();
    assert_eq!(decoded, transfer);
    let mut destination = Workspace::new(Document::new("Destination", 16, 12));
    destination.document.background = [0; 4];
    destination
        .execute(Command::InsertLayer {
            transfer: Box::new(decoded),
            index: None,
        })
        .unwrap();
    assert_eq!(
        render_document(&destination.document, None)
            .unwrap()
            .to_rgba8(),
        expected
    );

    let mut forged = transfer;
    forged.layer.pixel_mask.as_mut().unwrap().content_hash[0] ^= 0xff;
    let encoded = serde_json::to_string(&forged).unwrap();
    assert!(LayerTransfer::from_json(&encoded).is_err());
}

#[test]
fn transfers_preserve_paths_and_reusable_vector_masks() {
    let path = crate::PathGeometry::new(
        100,
        80,
        true,
        crate::PathFillRule::EvenOdd,
        vec![
            crate::PathAnchor::corner(50.0, 0.0),
            crate::PathAnchor::corner(100.0, 80.0),
            crate::PathAnchor::corner(0.0, 80.0),
        ],
    )
    .unwrap();
    let mut source = Document::new("Path transfer", 200, 160);
    source.layers.push(Layer {
        id: 1,
        vector_mask: Some(crate::VectorMask::new(path.clone(), true).unwrap()),
        kind: LayerKind::Path {
            geometry: path.clone(),
            color: [40, 120, 220, 200],
        },
        ..Layer::default()
    });
    let transfer = LayerTransfer::from_document(&source, 1).unwrap();
    let decoded = LayerTransfer::from_json(&transfer.to_json().unwrap()).unwrap();
    let LayerKind::Path { geometry, .. } = &decoded.layer.kind else {
        panic!("transfer lost path kind")
    };
    assert_eq!(geometry, &path);
    assert_eq!(decoded.layer.vector_mask.as_ref().unwrap().path, path);
}

#[test]
fn text_transfer_deduplicates_font_bytes_and_remaps_the_font_id() {
    let directory = test_directory("font-dedup");
    fs::create_dir_all(&directory).unwrap();
    let font_path = directory.join("Hack-Regular.ttf");
    fs::write(&font_path, epaint_default_fonts::HACK_REGULAR).unwrap();

    let mut source = Workspace::new(Document::new("Source", 500, 300));
    source
        .execute(Command::ImportFont {
            path: font_path.clone(),
            source_name: None,
        })
        .unwrap();
    source
        .execute(Command::AddText {
            text: "Portable type".into(),
            name: Some("Exact type".into()),
            font_size: 64.0,
            color: [220, 210, 180, 255],
            x: 80.0,
            y: 90.0,
            shaping: Default::default(),
        })
        .unwrap();
    let source_id = source.document.selected.unwrap();
    source
        .execute(Command::SetTextTypography {
            id: source_id,
            typography: TextTypography {
                font_id: Some(1),
                alignment: TextAlignment::Center,
                line_height: 1.4,
                tracking: 2.5,
                box_width: Some(280.0),
                shaping: Default::default(),
                effects: TextEffects {
                    outline_width: 2.0,
                    outline_color: [10, 11, 12, 255],
                    shadow_offset_x: 5.0,
                    shadow_offset_y: 7.0,
                    shadow_color: [13, 14, 15, 120],
                },
            },
        })
        .unwrap();
    let transfer = LayerTransfer::from_selected(&source.document).unwrap();
    assert!(transfer.font_asset.is_some());
    assert!(!transfer.to_json().unwrap().contains("original_path"));
    let LayerKind::Text { typography, .. } = &transfer.layer.kind else {
        panic!("transfer should stay text");
    };
    assert_eq!(typography.font_id, None);

    let mut destination_document = Document::new("Destination", 500, 300);
    destination_document.next_font_id = 17;
    let mut destination = Workspace::new(destination_document);
    destination
        .execute(Command::ImportFont {
            path: font_path.clone(),
            source_name: None,
        })
        .unwrap();
    assert_eq!(destination.document.font_assets[0].id, 17);
    let inserted = destination
        .execute(Command::InsertLayer {
            transfer: Box::new(transfer),
            index: Some(0),
        })
        .unwrap()
        .layer_ids[0];
    assert_eq!(destination.document.font_assets.len(), 1);
    let LayerKind::Text { typography, .. } = &destination.document.layer(inserted).unwrap().kind
    else {
        panic!("inserted layer should stay text");
    };
    assert_eq!(typography.font_id, Some(17));
    assert_eq!(destination.document.next_font_id, 18);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_text_transfer_embeds_font_bytes_and_replays_the_remapped_id() {
    let directory = test_directory("durable-font");
    fs::create_dir_all(&directory).unwrap();
    let font_path = directory.join("Hack-Regular.ttf");
    fs::write(&font_path, epaint_default_fonts::HACK_REGULAR).unwrap();
    let mut source = Workspace::new(Document::new("Source", 400, 200));
    source
        .execute(Command::ImportFont {
            path: font_path.clone(),
            source_name: None,
        })
        .unwrap();
    source
        .execute(Command::AddText {
            text: "Embedded".into(),
            name: None,
            font_size: 48.0,
            color: [255; 4],
            x: 20.0,
            y: 30.0,
            shaping: Default::default(),
        })
        .unwrap();
    let source_id = source.document.selected.unwrap();
    source
        .execute(Command::SetTextTypography {
            id: source_id,
            typography: TextTypography {
                font_id: Some(1),
                ..Default::default()
            },
        })
        .unwrap();
    let transfer = LayerTransfer::from_selected(&source.document).unwrap();

    let project_path = directory.join("destination.spectrum");
    let mut initial = Document::new("Destination", 400, 200);
    initial.next_font_id = 23;
    let mut destination = Workspace::create(
        &project_path,
        initial,
        test_actor(),
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    destination
        .execute(Command::InsertLayer {
            transfer: Box::new(transfer),
            index: None,
        })
        .unwrap();
    destination.checkpoint().unwrap();
    drop(destination);
    fs::remove_file(&font_path).unwrap();

    let loaded = Workspace::read(&project_path).unwrap();
    assert_eq!(loaded.font_assets.len(), 1);
    assert_eq!(loaded.font_assets[0].id, 23);
    assert!(loaded.font_assets[0].path.exists());
    assert_eq!(
        loaded.font_assets[0].bytes().unwrap(),
        epaint_default_fonts::HACK_REGULAR
    );
    let LayerKind::Text { typography, .. } = &loaded.layers[0].kind else {
        panic!("transferred layer should stay text");
    };
    assert_eq!(typography.font_id, Some(23));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn shaped_text_transfer_round_trips() {
    let mut workspace = Workspace::new(Document::new("Shaped transfer", 320, 180));
    workspace
        .execute(Command::AddText {
            text: "office العربية".into(),
            name: None,
            font_size: 42.0,
            color: [255; 4],
            x: 12.0,
            y: 18.0,
            shaping: crate::TextShaping::harfbuzz_v1(Some("ar")).unwrap(),
        })
        .unwrap();
    let transfer = LayerTransfer::from_selected(&workspace.document).unwrap();
    assert_eq!(transfer.version, crate::LAYER_TRANSFER_VERSION);
    assert_eq!(
        LayerTransfer::from_json(&transfer.to_json().unwrap()).unwrap(),
        transfer
    );
}
