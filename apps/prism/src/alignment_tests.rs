use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

fn rectangle(document: &mut Document, x: f32, y: f32, width: u32, height: u32) -> u64 {
    let mut workspace = Workspace::new(document.clone(), None);
    let id = workspace
        .execute(Command::AddRectangle {
            name: None,
            width,
            height,
            color: [255; 4],
            corner_radius: 0.0,
            x,
            y,
        })
        .unwrap()
        .layer_ids[0];
    *document = workspace.document;
    id
}

fn text(document: &mut Document, value: &str, x: f32, y: f32) -> u64 {
    let mut workspace = Workspace::new(document.clone(), None);
    let id = workspace
        .execute(Command::AddText {
            text: value.into(),
            name: None,
            font_size: 48.0,
            color: [255; 4],
            x,
            y,
        })
        .unwrap()
        .layer_ids[0];
    *document = workspace.document;
    id
}

#[test]
fn rotated_geometry_uses_the_actual_transformed_corners() {
    let layer = Layer {
        transform: Transform {
            x: 20.0,
            y: 30.0,
            scale_x: 2.0,
            scale_y: 1.0,
            rotation: 90.0,
        },
        kind: LayerKind::Rectangle {
            width: 100,
            height: 40,
            color: [255; 4],
            corner_radius: 0.0,
        },
        ..Default::default()
    };
    let geometry = layer_geometry(&layer).unwrap();
    assert!((geometry.min[0] - 100.0).abs() < 0.001);
    assert!((geometry.max[0] - 140.0).abs() < 0.001);
    assert!((geometry.min[1] + 50.0).abs() < 0.001);
    assert!((geometry.max[1] - 150.0).abs() < 0.001);
    assert_eq!(geometry.center, [120.0, 50.0]);
}

#[test]
fn shape_strokes_keep_the_same_geometry_because_they_render_inside_the_source_envelope() {
    let mut layer = Layer {
        transform: Transform {
            x: 20.0,
            y: 30.0,
            rotation: 23.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let without_stroke = layer_geometry(&layer).unwrap();
    layer.stroke = ShapeStroke {
        enabled: true,
        width: 24.0,
        color: [0, 0, 0, 255],
    };
    assert_eq!(layer_geometry(&layer).unwrap(), without_stroke);
}

#[test]
fn align_to_canvas_uses_rotated_visual_bounds_without_changing_rotation() {
    let mut document = Document::new("Alignment", 500, 400);
    let id = rectangle(&mut document, 80.0, 70.0, 120, 40);
    document.layer_mut(id).unwrap().transform.rotation = 30.0;
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::AlignLayer {
            id,
            alignment: Alignment::Left,
            reference: AlignmentReference::Canvas,
        })
        .unwrap();
    let layer = workspace.document.layer(id).unwrap();
    assert_eq!(layer.transform.rotation, 30.0);
    assert!(layer_geometry(layer).unwrap().min[0].abs() < 0.001);
}

#[test]
fn align_to_another_rotated_layer_and_undo_are_exact() {
    let mut document = Document::new("Alignment", 800, 600);
    let moving = rectangle(&mut document, 20.0, 30.0, 100, 40);
    let reference = rectangle(&mut document, 420.0, 240.0, 80, 160);
    document.layer_mut(moving).unwrap().transform.rotation = 27.0;
    document.layer_mut(reference).unwrap().transform.rotation = 315.0;
    let before = document.layer(moving).unwrap().transform;
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::AlignLayer {
            id: moving,
            alignment: Alignment::VerticalCenter,
            reference: AlignmentReference::Layer { id: reference },
        })
        .unwrap();
    let moving_geometry = layer_geometry(workspace.document.layer(moving).unwrap()).unwrap();
    let reference_geometry = layer_geometry(workspace.document.layer(reference).unwrap()).unwrap();
    assert!((moving_geometry.center[1] - reference_geometry.center[1]).abs() < 0.001);
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(workspace.document.layer(moving).unwrap().transform, before);
}

#[test]
fn text_alignment_uses_visible_glyph_bounds_for_canvas_and_layer_references() {
    let mut document = Document::new("Text alignment", 800, 600);
    let moving = text(&mut document, "Visible", 80.0, 70.0);
    let reference = text(&mut document, "Reference", 420.0, 240.0);
    document.layer_mut(moving).unwrap().transform.rotation = 19.0;
    document.layer_mut(reference).unwrap().transform.rotation = 331.0;
    let mut workspace = Workspace::new(document, None);

    workspace
        .execute(Command::AlignLayer {
            id: moving,
            alignment: Alignment::Top,
            reference: AlignmentReference::Canvas,
        })
        .unwrap();
    let moving_geometry = layer_geometry(workspace.document.layer(moving).unwrap()).unwrap();
    assert!(moving_geometry.min[1].abs() < 0.001);

    workspace
        .execute(Command::AlignLayer {
            id: moving,
            alignment: Alignment::Right,
            reference: AlignmentReference::Layer { id: reference },
        })
        .unwrap();
    let moving_geometry = layer_geometry(workspace.document.layer(moving).unwrap()).unwrap();
    let reference_geometry = layer_geometry(workspace.document.layer(reference).unwrap()).unwrap();
    assert!((moving_geometry.max[0] - reference_geometry.max[0]).abs() < 0.001);
}

#[test]
fn imported_typography_alignment_uses_the_rendered_effect_bounds() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let font_path = std::env::temp_dir().join(format!("prism-align-font-{stamp}.ttf"));
    std::fs::write(&font_path, epaint_default_fonts::HACK_REGULAR).unwrap();
    let font = FontAsset::import(31, &font_path).unwrap();
    let mut document = Document::new("Imported alignment", 720, 420);
    document.font_assets.push(font);
    let id = text(&mut document, "Aligned imported face", 63.0, 48.0);
    let LayerKind::Text { typography, .. } = &mut document.layer_mut(id).unwrap().kind else {
        unreachable!();
    };
    *typography = TextTypography {
        font_id: Some(31),
        alignment: TextAlignment::Right,
        tracking: 2.0,
        box_width: Some(360.0),
        effects: TextEffects {
            outline_width: 3.0,
            shadow_offset_x: -9.0,
            shadow_offset_y: 7.0,
            shadow_color: [0, 0, 0, 160],
            ..Default::default()
        },
        ..Default::default()
    };
    document.layer_mut(id).unwrap().transform.rotation = 14.0;
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::AlignLayer {
            id,
            alignment: Alignment::Right,
            reference: AlignmentReference::Canvas,
        })
        .unwrap();
    let layer = workspace.document.layer(id).unwrap();
    let LayerKind::Text {
        text,
        font_size,
        typography,
        ..
    } = &layer.kind
    else {
        unreachable!();
    };
    let geometry = measure_text_geometry_with_typography(
        text,
        *font_size,
        typography,
        workspace.document.font_for_layer(layer),
    )
    .unwrap();
    let transformed = layer_geometry_with_bounds(
        layer,
        [geometry.visual_left, geometry.visual_top],
        [geometry.visual_width, geometry.visual_height],
    );
    let _ = std::fs::remove_file(font_path);
    assert!((transformed.max[0] - workspace.document.width as f32).abs() < 0.001);
}

#[test]
fn alignment_respects_locking_and_rejects_self_reference() {
    let mut document = Document::new("Alignment", 500, 400);
    let id = rectangle(&mut document, 80.0, 70.0, 120, 40);
    document.layer_mut(id).unwrap().locked = true;
    let mut workspace = Workspace::new(document, None);
    assert!(
        workspace
            .execute(Command::AlignLayer {
                id,
                alignment: Alignment::Left,
                reference: AlignmentReference::Canvas,
            })
            .unwrap_err()
            .to_string()
            .contains("locked")
    );
    workspace.document.layer_mut(id).unwrap().locked = false;
    assert!(
        workspace
            .execute(Command::AlignLayer {
                id,
                alignment: Alignment::Left,
                reference: AlignmentReference::Layer { id },
            })
            .unwrap_err()
            .to_string()
            .contains("itself")
    );
}

#[test]
fn guide_commands_clamp_validate_and_share_one_step_gestures() {
    let mut workspace = Workspace::new(Document::new("Guides", 400, 300), None);
    let output = workspace
        .execute(Command::AddGuide {
            orientation: GuideOrientation::Vertical,
            position: 450.0,
        })
        .unwrap();
    let id = output.guide_ids[0];
    assert_eq!(workspace.document.guide(id).unwrap().position, 400.0);

    workspace.begin_interaction();
    for position in [320.0, 210.0, 123.0] {
        workspace
            .preview(Command::MoveGuide { id, position })
            .unwrap();
    }
    assert!(workspace.commit_interaction().unwrap());
    assert_eq!(workspace.document.guide(id).unwrap().position, 123.0);
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(workspace.document.guide(id).unwrap().position, 400.0);
    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.guides.is_empty());

    assert!(
        workspace
            .execute(Command::AddGuide {
                orientation: GuideOrientation::Horizontal,
                position: f32::NAN,
            })
            .is_err()
    );
}

#[test]
fn snapped_move_previews_commit_as_exactly_one_history_revision() {
    let mut document = Document::new("Snapping", 500, 400);
    let id = rectangle(&mut document, 40.0, 60.0, 100, 80);
    let before = document.layer(id).unwrap().transform;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("prism-snapping-history-{stamp}"));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("snapping.prism");
    let mut workspace = Workspace::create_durable(
        document,
        &project,
        spectrum_revisions::Actor {
            id: "test:snapping".into(),
            display_name: "Snapping Test".into(),
            kind: spectrum_revisions::ActorKind::Human,
        },
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    assert_eq!(workspace.history().unwrap().unwrap().revisions.len(), 1);
    workspace.begin_interaction();
    for x in [110.0, 180.0, 200.0] {
        workspace
            .preview(Command::SetTransform {
                id,
                transform: Transform { x, ..before },
            })
            .unwrap();
    }
    assert!(workspace.commit_interaction().unwrap());
    assert_eq!(workspace.history().unwrap().unwrap().revisions.len(), 2);
    assert_eq!(workspace.document.layer(id).unwrap().transform.x, 200.0);
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(workspace.document.layer(id).unwrap().transform, before);
    assert!(workspace.execute(Command::Undo).is_err());
    workspace.execute(Command::Redo).unwrap();
    assert_eq!(workspace.document.layer(id).unwrap().transform.x, 200.0);
    drop(workspace);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn guide_drag_changes_only_the_guide_and_commits_one_revision() {
    let mut document = Document::new("Guide drag", 500, 400);
    let layer_id = rectangle(&mut document, 40.0, 60.0, 100, 80);
    document.guides.push(Guide {
        id: 1,
        orientation: GuideOrientation::Vertical,
        position: 100.0,
    });
    document.next_guide_id = 2;
    let layer_before = document.layer(layer_id).unwrap().transform;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("prism-guide-drag-{stamp}"));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("guide-drag.prism");
    let mut workspace = Workspace::create_durable(
        document,
        &project,
        spectrum_revisions::Actor {
            id: "test:guide-drag".into(),
            display_name: "Guide Drag Test".into(),
            kind: spectrum_revisions::ActorKind::Human,
        },
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();

    workspace.begin_interaction();
    for position in [120.0, 180.0, 240.0] {
        workspace
            .preview(Command::MoveGuide { id: 1, position })
            .unwrap();
    }
    assert!(workspace.commit_interaction().unwrap());
    assert_eq!(workspace.history().unwrap().unwrap().revisions.len(), 2);
    assert_eq!(workspace.document.guide(1).unwrap().position, 240.0);
    assert_eq!(
        workspace.document.layer(layer_id).unwrap().transform,
        layer_before
    );
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(workspace.document.guide(1).unwrap().position, 100.0);
    assert_eq!(
        workspace.document.layer(layer_id).unwrap().transform,
        layer_before
    );
    drop(workspace);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn guides_and_snapping_round_trip_in_the_one_file_project() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("prism-guides-{stamp}.prism"));
    let document = Document {
        guides: vec![Guide {
            id: 7,
            orientation: GuideOrientation::Horizontal,
            position: 86.5,
        }],
        snapping_enabled: false,
        next_guide_id: 8,
        ..Document::new("Guides", 320, 240)
    };
    save_document(&document, &path).unwrap();
    let loaded = load_document(&path).unwrap();
    assert_eq!(loaded.guides, document.guides);
    assert!(!loaded.snapping_enabled);
    assert_eq!(loaded.next_guide_id, 8);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn crop_repositions_and_discards_guides_outside_the_new_canvas() {
    let mut document = Document::new("Guides", 400, 300);
    document.guides = vec![
        Guide {
            id: 1,
            orientation: GuideOrientation::Vertical,
            position: 30.0,
        },
        Guide {
            id: 2,
            orientation: GuideOrientation::Vertical,
            position: 180.0,
        },
        Guide {
            id: 3,
            orientation: GuideOrientation::Horizontal,
            position: 100.0,
        },
    ];
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::CropCanvas {
            x: 50,
            y: 20,
            width: 200,
            height: 160,
        })
        .unwrap();
    assert_eq!(workspace.document.guides.len(), 2);
    assert_eq!(workspace.document.guide(2).unwrap().position, 130.0);
    assert_eq!(workspace.document.guide(3).unwrap().position, 80.0);
}
