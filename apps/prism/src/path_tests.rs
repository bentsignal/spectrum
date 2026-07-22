use std::time::{SystemTime, UNIX_EPOCH};

use image::imageops;

use crate::*;

fn corner_path(width: u32, height: u32, closed: bool) -> PathGeometry {
    let anchors = if closed {
        vec![
            PathAnchor::corner(2.0, 2.0),
            PathAnchor::corner(width as f32 - 2.0, 2.0),
            PathAnchor::corner(width as f32 * 0.5, height as f32 - 2.0),
        ]
    } else {
        vec![
            PathAnchor::corner(2.0, height as f32 - 2.0),
            PathAnchor {
                point: [width as f32 * 0.5, 2.0],
                handle_in: [-(width as f32) * 0.22, 0.0],
                handle_out: [width as f32 * 0.22, 0.0],
            },
            PathAnchor::corner(width as f32 - 2.0, height as f32 - 2.0),
        ]
    };
    PathGeometry::new(width, height, closed, PathFillRule::EvenOdd, anchors).unwrap()
}

fn test_project(label: &str) -> std::path::PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-path-{label}-{stamp}.prism"))
}

#[test]
fn path_commands_are_atomic_undoable_and_durable() {
    let project = test_project("history");
    let actor = spectrum_revisions::Actor {
        id: "person:path-test".into(),
        display_name: "Path test".into(),
        kind: spectrum_revisions::ActorKind::Human,
    };
    let session = spectrum_revisions::SessionId::new();
    let mut workspace = Workspace::create_durable(
        Document::new("Paths", 320, 240),
        &project,
        actor.clone(),
        session,
    )
    .unwrap();
    let original = corner_path(100, 80, false);
    workspace
        .execute(Command::AddPath {
            name: Some("Curve".into()),
            geometry: original.clone(),
            color: [240, 220, 200, 255],
            x: 30.0,
            y: 40.0,
        })
        .unwrap();
    let edited = original
        .replacing_anchor(1, PathAnchor::corner(50.0, 18.0))
        .unwrap();
    workspace
        .execute(Command::ReplacePath {
            id: 1,
            geometry: edited.clone(),
        })
        .unwrap();
    workspace.execute(Command::Undo).unwrap();
    let LayerKind::Path { geometry, .. } = &workspace.document.layer(1).unwrap().kind else {
        panic!("expected path")
    };
    assert_eq!(geometry, &original);
    workspace.execute(Command::Redo).unwrap();
    workspace.save(None).unwrap();
    drop(workspace);

    let reopened = Workspace::open_as(&project, actor, session).unwrap();
    let LayerKind::Path { geometry, .. } = &reopened.document.layer(1).unwrap().kind else {
        panic!("expected path")
    };
    assert_eq!(geometry, &edited);
    drop(reopened);
    std::fs::remove_file(project).unwrap();
}

#[test]
fn multi_preview_anchor_gesture_commits_one_durable_revision() {
    let project = test_project("gesture-history");
    let actor = spectrum_revisions::Actor {
        id: "person:path-gesture".into(),
        display_name: "Path gesture".into(),
        kind: spectrum_revisions::ActorKind::Human,
    };
    let session = spectrum_revisions::SessionId::new();
    let mut workspace = Workspace::create_durable(
        Document::new("Gesture", 320, 240),
        &project,
        actor.clone(),
        session,
    )
    .unwrap();
    let original = corner_path(100, 80, false);
    workspace
        .execute(Command::AddPath {
            name: None,
            geometry: original.clone(),
            color: [255; 4],
            x: 20.0,
            y: 30.0,
        })
        .unwrap();
    let before = workspace.document.clone();
    workspace.begin_interaction();
    let middle = original
        .replacing_anchor(1, PathAnchor::corner(48.0, 12.0))
        .unwrap();
    workspace
        .preview_batch(vec![
            Command::ReplacePath {
                id: 1,
                geometry: middle,
            },
            Command::SetTransform {
                id: 1,
                transform: Transform {
                    x: 18.0,
                    y: 29.0,
                    ..Transform::default()
                },
            },
        ])
        .unwrap();
    let final_geometry = original
        .replacing_anchor(
            1,
            PathAnchor {
                point: [44.0, 9.0],
                handle_in: [-12.0, 3.0],
                handle_out: [14.0, -2.0],
            },
        )
        .unwrap();
    let final_transform = Transform {
        x: 16.0,
        y: 27.0,
        ..Transform::default()
    };
    workspace
        .preview_batch(vec![
            Command::ReplacePath {
                id: 1,
                geometry: final_geometry.clone(),
            },
            Command::SetTransform {
                id: 1,
                transform: final_transform,
            },
        ])
        .unwrap();
    assert!(workspace.commit_interaction().unwrap());
    let final_document = workspace.document.clone();

    workspace.execute(Command::Undo).unwrap();
    assert_eq!(workspace.document, before);
    workspace.execute(Command::Redo).unwrap();
    assert_eq!(workspace.document, final_document);
    workspace.save(None).unwrap();
    drop(workspace);
    let reopened = Workspace::open_as(&project, actor, session).unwrap();
    assert_eq!(reopened.document, final_document);
    let LayerKind::Path { geometry, .. } = &reopened.document.layer(1).unwrap().kind else {
        panic!("expected path")
    };
    assert_eq!(geometry, &final_geometry);
    assert_eq!(
        reopened.document.layer(1).unwrap().transform,
        final_transform
    );
    drop(reopened);
    std::fs::remove_file(project).unwrap();
}

#[test]
fn open_paths_are_stroke_only_and_closed_paths_fill() {
    let mut open = Layer {
        kind: LayerKind::Path {
            geometry: corner_path(40, 30, false),
            color: [220, 40, 60, 255],
        },
        stroke: ShapeStroke {
            enabled: true,
            width: 3.0,
            color: [40, 100, 240, 255],
        },
        ..Layer::default()
    };
    let open_image = render_layer_base(&open, None).unwrap().to_rgba8();
    assert_eq!(open_image.get_pixel(open_image.width() / 2, 18)[3], 0);
    assert!(
        open_image
            .pixels()
            .any(|pixel| pixel[2] > 200 && pixel[3] > 0)
    );

    let LayerKind::Path { geometry, .. } = &mut open.kind else {
        unreachable!()
    };
    *geometry = corner_path(40, 30, true);
    let closed_image = render_layer_base(&open, None).unwrap().to_rgba8();
    assert_eq!(closed_image.get_pixel(closed_image.width() / 2, 12)[3], 255);
}

#[test]
fn path_regions_match_export_with_rotation_scale_gradient_and_alpha() {
    let mut document = Document::new("Path region parity", 180, 140);
    document.background = [15, 24, 35, 123];
    document.layers.push(Layer {
        id: 1,
        opacity: 0.73,
        adjustments: spectrum_imaging::Adjustments {
            rotation: 90,
            contrast: 8.0,
            ..Default::default()
        },
        transform: Transform {
            x: 33.0,
            y: 24.0,
            scale_x: 1.37,
            scale_y: 0.82,
            rotation: 17.0,
        },
        shape_fill: Some(ShapeFill::Gradient(ShapeGradient {
            angle: 41.0,
            stops: vec![
                GradientStop::new(0.0, [230, 40, 80, 190]),
                GradientStop::new(1.0, [30, 210, 180, 115]),
            ],
            ..ShapeGradient::default()
        })),
        stroke: ShapeStroke {
            enabled: true,
            width: 7.0,
            color: [245, 230, 120, 177],
        },
        kind: LayerKind::Path {
            geometry: corner_path(72, 58, true),
            color: [255; 4],
        },
        ..Layer::default()
    });
    let full = render_document_scaled(&document, 1.5).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 19,
        y: 13,
        width: 157,
        height: 121,
    };
    let tile = render_document_region_scaled(&document, 1.5, region)
        .unwrap()
        .to_rgba8();
    let oracle =
        imageops::crop_imm(&full, region.x, region.y, region.width, region.height).to_image();
    assert_eq!(tile, oracle);
}

#[test]
fn vector_masks_stretch_to_target_aspect_and_invert_soft_edges() {
    let geometry = corner_path(100, 100, true);
    let mut layer = Layer {
        kind: LayerKind::Rectangle {
            width: 80,
            height: 24,
            color: [80, 160, 240, 255],
            corner_radius: 0.0,
        },
        vector_mask: Some(VectorMask::new(geometry.clone(), false).unwrap()),
        ..Layer::default()
    };
    let normal = render_layer_preview(&layer, None).unwrap().to_rgba8();
    assert_eq!(normal.dimensions(), (80, 24));
    assert_eq!(normal.get_pixel(1, 1)[3], 0);
    assert_eq!(normal.get_pixel(40, 10)[3], 255);
    assert!(normal.pixels().any(|pixel| matches!(pixel[3], 1..=254)));

    layer.vector_mask = Some(VectorMask::new(geometry, true).unwrap());
    let inverted = render_layer_preview(&layer, None).unwrap().to_rgba8();
    for (left, right) in normal.pixels().zip(inverted.pixels()) {
        assert!((i16::from(left[3]) + i16::from(right[3]) - 255).abs() <= 1);
    }
}

#[test]
fn vector_pixel_rectangle_masks_and_shadow_keep_exact_region_parity() {
    let geometry = corner_path(100, 100, true);
    let mut alpha = Vec::with_capacity(80 * 48);
    for y in 0..48_u32 {
        for x in 0..80_u32 {
            alpha.push(((x * 3 + y * 5) % 256) as u8);
        }
    }
    let mut document = Document::new("Combined masks", 190, 150);
    document.background = [20, 30, 50, 160];
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            x: 42.0,
            y: 31.0,
            scale_x: 1.42,
            scale_y: 0.91,
            rotation: -19.0,
        },
        mask: LayerMask {
            enabled: true,
            x: 0.08,
            y: 0.12,
            width: 0.79,
            height: 0.72,
            invert: false,
        },
        pixel_mask: Some(PixelMask::new(80, 48, alpha)),
        vector_mask: Some(VectorMask::new(geometry, false).unwrap()),
        style: LayerStyle {
            drop_shadow: Some(DropShadow {
                color: [0, 0, 0, 150],
                offset_x: 8.0,
                offset_y: 5.0,
                blur_radius: 7.0,
            }),
        },
        kind: LayerKind::Rectangle {
            width: 80,
            height: 48,
            color: [220, 80, 130, 210],
            corner_radius: 3.0,
        },
        ..Layer::default()
    });
    let full = render_document_scaled(&document, 1.25).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 17,
        y: 11,
        width: 173,
        height: 137,
    };
    let tile = render_document_region_scaled(&document, 1.25, region)
        .unwrap()
        .to_rgba8();
    assert_eq!(
        tile,
        imageops::crop_imm(&full, region.x, region.y, region.width, region.height).to_image()
    );
}

#[test]
fn stroke_padding_preserves_path_viewport_origin_and_rasterized_placement() {
    let geometry = corner_path(30, 20, true);
    let mut document = Document::new("Stroke placement", 120, 100);
    document.background = [0; 4];
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            x: 25.0,
            y: 31.0,
            rotation: 23.0,
            ..Transform::default()
        },
        stroke: ShapeStroke {
            enabled: true,
            width: 9.0,
            color: [255, 240, 180, 255],
        },
        adjustments: spectrum_imaging::Adjustments {
            rotation: 90,
            exposure: 0.2,
            ..Default::default()
        },
        kind: LayerKind::Path {
            geometry,
            color: [40, 110, 230, 255],
        },
        ..Layer::default()
    });
    let bounds = path_source_bounds(document.layer(1).unwrap()).unwrap();
    assert_eq!(bounds.origin, [-6.0, -6.0]);
    let before = render_document(&document, None).unwrap().to_rgba8();
    let asset = rasterize_shape_asset(&document, 1, 1.0).unwrap();
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::RasterizeShape {
            id: 1,
            path: asset.path.clone(),
            scale: asset.scale,
        })
        .unwrap();
    let after = render_document(&workspace.document, None)
        .unwrap()
        .to_rgba8();
    assert_eq!(after, before);
    std::fs::remove_file(asset.path).unwrap();
}
