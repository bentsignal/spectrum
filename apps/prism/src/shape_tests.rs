use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-{label}-{stamp}"))
}

#[test]
fn editable_shapes_render_fill_and_inside_strokes() {
    let mut workspace = Workspace::new(Document::new("Shapes", 64, 40), None);
    workspace.document.background = [0, 0, 0, 0];
    workspace
        .execute(Command::AddEllipse {
            name: Some("Badge".into()),
            width: 24,
            height: 24,
            color: [220, 40, 60, 255],
            x: 4.0,
            y: 4.0,
        })
        .unwrap();
    let ellipse = workspace.document.selected.unwrap();
    workspace
        .execute(Command::SetShapeStroke {
            id: ellipse,
            stroke: ShapeStroke {
                enabled: true,
                width: 3.0,
                color: [20, 30, 220, 255],
            },
        })
        .unwrap();

    let rendered = render_document(&workspace.document, None)
        .unwrap()
        .to_rgba8();
    assert_eq!(rendered.get_pixel(4, 4)[3], 0, "ellipse corners stay clear");
    assert_eq!(rendered.get_pixel(16, 4).0, [20, 30, 220, 255]);
    assert_eq!(rendered.get_pixel(16, 16).0, [220, 40, 60, 255]);
}

#[test]
fn durable_workspace_round_trips_ellipse_and_stroke() {
    let directory = test_directory("durable-shapes");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("shapes.prism");
    let session = spectrum_revisions::SessionId::new();
    let actor = spectrum_revisions::Actor {
        id: "person:shapes".into(),
        display_name: "Shape tester".into(),
        kind: spectrum_revisions::ActorKind::Human,
    };
    let mut workspace = Workspace::create_durable(
        Document::new("Shapes", 400, 300),
        &project_path,
        actor.clone(),
        session,
    )
    .unwrap();
    workspace
        .execute(Command::AddEllipse {
            name: Some("Orbit".into()),
            width: 180,
            height: 120,
            color: [10, 20, 30, 255],
            x: 40.0,
            y: 60.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetShapeStroke {
            id: 1,
            stroke: ShapeStroke {
                enabled: true,
                width: 7.5,
                color: [240, 230, 220, 255],
            },
        })
        .unwrap();
    drop(workspace);

    let reopened = Workspace::open_as(&project_path, actor, session).unwrap();
    let layer = reopened.document.layer(1).unwrap();
    assert!(matches!(
        layer.kind,
        LayerKind::Ellipse {
            width: 180,
            height: 120,
            ..
        }
    ));
    assert_eq!(layer.stroke.width, 7.5);
    assert_eq!(layer.stroke.color, [240, 230, 220, 255]);
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn scaled_shape_rendering_regenerates_geometry_at_output_resolution() {
    let mut document = Document::new("Scaled geometry", 80, 80);
    document.background = [0, 0, 0, 0];
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::AddEllipse {
            name: Some("Vector circle".into()),
            width: 8,
            height: 8,
            color: [210, 80, 40, 255],
            x: 8.0,
            y: 8.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetTransform {
            id: 1,
            transform: Transform {
                x: 8.0,
                y: 8.0,
                scale_x: 8.0,
                scale_y: 8.0,
                ..Default::default()
            },
        })
        .unwrap();

    let layer = workspace.document.layer(1).unwrap();
    let geometry = render_layer_base_scaled(layer, None, [8.0, 8.0])
        .unwrap()
        .to_rgba8();
    let enlarged_natural = render_layer_base(layer, None)
        .unwrap()
        .resize_exact(64, 64, image::imageops::FilterType::Triangle)
        .to_rgba8();
    assert_eq!(geometry.dimensions(), (64, 64));
    assert_ne!(geometry, enlarged_natural);
    assert!(geometry.pixels().any(|pixel| matches!(pixel[3], 1..=254)));

    let rendered = render_document(&workspace.document, None)
        .unwrap()
        .to_rgba8();
    for y in 0..64 {
        for x in 0..64 {
            assert_eq!(rendered.get_pixel(x + 8, y + 8), geometry.get_pixel(x, y));
        }
    }
    assert!(matches!(
        workspace.document.layer(1).unwrap().kind,
        LayerKind::Ellipse {
            width: 8,
            height: 8,
            color: [210, 80, 40, 255]
        }
    ));
}

#[test]
fn rasterize_shape_freezes_pixels_and_is_undoable() {
    let mut workspace = Workspace::new(Document::new("Rasterize", 100, 100), None);
    workspace
        .execute(Command::AddRectangle {
            name: Some("Editable badge".into()),
            width: 12,
            height: 8,
            color: [12, 80, 210, 255],
            corner_radius: 3.0,
            x: 4.0,
            y: 5.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetTransform {
            id: 1,
            transform: Transform {
                x: 4.0,
                y: 5.0,
                scale_x: 3.0,
                scale_y: 3.0,
                ..Default::default()
            },
        })
        .unwrap();
    let asset = rasterize_shape_asset(&workspace.document, 1, 3.0).unwrap();
    workspace
        .execute(Command::RasterizeShape {
            id: 1,
            path: asset.path.clone(),
            scale: asset.scale,
        })
        .unwrap();
    let raster = workspace.document.layer(1).unwrap();
    assert!(matches!(raster.kind, LayerKind::Raster { .. }));
    assert_eq!(image::image_dimensions(asset.path).unwrap(), (36, 24));
    assert_eq!(raster.transform.scale_x, 1.0);
    assert_eq!(raster.transform.scale_y, 1.0);

    workspace.execute(Command::Undo).unwrap();
    assert!(matches!(
        workspace.document.layer(1).unwrap().kind,
        LayerKind::Rectangle {
            width: 12,
            height: 8,
            corner_radius: 3.0,
            ..
        }
    ));
    workspace.execute(Command::Redo).unwrap();
    assert!(matches!(
        workspace.document.layer(1).unwrap().kind,
        LayerKind::Raster { .. }
    ));
}

#[test]
fn masked_shape_dimension_edits_fail_atomically() {
    let mut workspace = Workspace::new(Document::new("Masked edits", 40, 30), None);
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 3,
            height: 2,
            color: [255; 4],
            corner_radius: 0.0,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    workspace.document.layer_mut(1).unwrap().pixel_mask = Some(PixelMask::new(3, 2, vec![255; 6]));
    let before = workspace.document.clone();
    assert!(
        workspace
            .execute(Command::UpdateRectangle {
                id: 1,
                width: 4,
                height: 2,
                color: [10, 20, 30, 255],
                corner_radius: 0.0,
            })
            .is_err()
    );
    assert_eq!(workspace.document, before);

    workspace.document.layers[0].kind = LayerKind::Ellipse {
        width: 3,
        height: 2,
        color: [255; 4],
    };
    let before = workspace.document.clone();
    assert!(
        workspace
            .execute(Command::UpdateEllipse {
                id: 1,
                width: 3,
                height: 4,
                color: [10, 20, 30, 255],
            })
            .is_err()
    );
    assert_eq!(workspace.document, before);
}

#[test]
fn rasterizing_a_masked_shape_bakes_and_clears_the_mask_across_reopen() {
    let directory = test_directory("durable-masked-rasterized-shape");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("masked-rasterized.prism");
    let actor = spectrum_revisions::Actor {
        id: "person:masked-rasterize".into(),
        display_name: "Masked rasterize tester".into(),
        kind: spectrum_revisions::ActorKind::Human,
    };
    let session = spectrum_revisions::SessionId::new();
    let mut initial = Document::new("Masked rasterize", 12, 8);
    initial.background = [0; 4];
    let mut workspace =
        Workspace::create_durable(initial, &project_path, actor.clone(), session).unwrap();
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::color_mask(
                2,
                3,
                4,
                2,
                vec![255, 0, 255, 0, 0, 255, 0, 255],
            )),
        })
        .unwrap();
    workspace
        .execute(Command::FillSelection {
            color: [30, 100, 220, 255],
            name: Some("Masked fill".into()),
        })
        .unwrap();
    let before = render_document(&workspace.document, None)
        .unwrap()
        .to_rgba8();
    let asset = rasterize_shape_asset(&workspace.document, 1, 1.0).unwrap();
    workspace
        .execute(Command::RasterizeShape {
            id: 1,
            path: asset.path,
            scale: asset.scale,
        })
        .unwrap();
    assert!(workspace.document.layer(1).unwrap().pixel_mask.is_none());
    assert_eq!(
        render_document(&workspace.document, None)
            .unwrap()
            .to_rgba8(),
        before
    );
    drop(workspace);

    let reopened = Workspace::open_as(&project_path, actor, session).unwrap();
    assert!(reopened.document.layer(1).unwrap().pixel_mask.is_none());
    assert!(matches!(
        reopened.document.layer(1).unwrap().kind,
        LayerKind::Raster { .. }
    ));
    assert_eq!(
        render_document(&reopened.document, None)
            .unwrap()
            .to_rgba8(),
        before
    );
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_rasterization_embeds_pixels_and_replays_history() {
    let directory = test_directory("durable-rasterized-shape");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("rasterized.prism");
    let session = spectrum_revisions::SessionId::new();
    let actor = spectrum_revisions::Actor {
        id: "person:rasterize".into(),
        display_name: "Rasterize tester".into(),
        kind: spectrum_revisions::ActorKind::Human,
    };
    let mut workspace = Workspace::create_durable(
        Document::new("Durable rasterize", 320, 240),
        &project_path,
        actor.clone(),
        session,
    )
    .unwrap();
    workspace
        .execute(Command::AddEllipse {
            name: Some("Editable orbit".into()),
            width: 24,
            height: 16,
            color: [30, 180, 120, 255],
            x: 20.0,
            y: 30.0,
        })
        .unwrap();
    let asset = rasterize_shape_asset(&workspace.document, 1, 2.0).unwrap();
    workspace
        .execute(Command::RasterizeShape {
            id: 1,
            path: asset.path,
            scale: asset.scale,
        })
        .unwrap();
    drop(workspace);

    let mut reopened = Workspace::open_as(&project_path, actor, session).unwrap();
    let LayerKind::Raster { path, .. } = &reopened.document.layer(1).unwrap().kind else {
        panic!("rasterized shape did not replay");
    };
    assert!(path.exists());
    assert_eq!(image::image_dimensions(path).unwrap(), (48, 32));
    reopened.execute(Command::Undo).unwrap();
    assert!(matches!(
        reopened.document.layer(1).unwrap().kind,
        LayerKind::Ellipse {
            width: 24,
            height: 16,
            ..
        }
    ));
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn legacy_shape_json_without_stroke_keeps_editable_geometry() {
    let directory = test_directory("legacy-shape-json");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("legacy.mica");
    std::fs::write(
        &project_path,
        r#"{
            "version": 1,
            "name": "Legacy shape",
            "width": 320,
            "height": 200,
            "background": [0,0,0,0],
            "layers": [{
                "id": 1,
                "name": "Old rectangle",
                "visible": true,
                "locked": false,
                "opacity": 1.0,
                "blend_mode": "normal",
                "transform": {"x": 4.0, "y": 5.0, "scale_x": 2.0, "scale_y": 2.0, "rotation": 0.0},
                "adjustments": {},
                "mask": {},
                "clip_to_below": false,
                "kind": {"type": "rectangle", "width": 40, "height": 20, "color": [1,2,3,255], "corner_radius": 6.0}
            }],
            "selected": 1,
            "next_id": 2
        }"#,
    )
    .unwrap();
    let document = load_document(&project_path).unwrap();
    assert!(matches!(
        document.layer(1).unwrap().kind,
        LayerKind::Rectangle {
            width: 40,
            height: 20,
            corner_radius: 6.0,
            ..
        }
    ));
    assert_eq!(document.layer(1).unwrap().stroke, ShapeStroke::default());
    std::fs::remove_dir_all(directory).unwrap();
}
