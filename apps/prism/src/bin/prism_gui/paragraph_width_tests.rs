use super::*;

fn paragraph_layer(transform: Transform, width: f32) -> Layer {
    Layer {
        id: 7,
        transform,
        kind: LayerKind::Text {
            text: "One two three four five six seven".into(),
            font_size: 32.0,
            color: [11, 22, 33, 244],
            typography: prism_core::TextTypography {
                alignment: prism_core::TextAlignment::Center,
                line_height: 1.7,
                tracking: 2.5,
                box_width: Some(width),
                effects: prism_core::TextEffects {
                    outline_width: 2.0,
                    outline_color: [9, 8, 7, 255],
                    shadow_offset_x: 5.0,
                    shadow_offset_y: -3.0,
                    shadow_color: [6, 5, 4, 128],
                },
                ..Default::default()
            },
        },
        ..Default::default()
    }
}

fn source_for(layer: &Layer) -> LayerSourceGeometry {
    let LayerKind::Text {
        text,
        font_size,
        typography,
        ..
    } = &layer.kind
    else {
        panic!("test layer must be text");
    };
    let geometry =
        prism_core::measure_text_geometry_with_typography(text, *font_size, typography, None)
            .unwrap();
    text_source_geometry(geometry, typography.box_width.is_some())
}

fn width_drag(layer: &Layer, source: LayerSourceGeometry, handle: ResizeHandle) -> DragState {
    let [left, right] = paragraph_width_handle_positions(layer, Some(source)).unwrap();
    let start = match handle {
        ResizeHandle::ParagraphLeft => left,
        ResizeHandle::ParagraphRight => right,
        _ => panic!("test requires a paragraph handle"),
    };
    DragState {
        start_canvas: start,
        current_canvas: start,
        layer_id: Some(layer.id),
        transform: layer.transform,
        action: DragAction::Resize(handle),
        bounds: layer_bounds(layer, Some(source)),
        paragraph_bounds: paragraph_layer_bounds(layer, Some(source)),
        paragraph_width: paragraph_box_width(layer),
        paragraph_source_override: None,
    }
}

#[test]
fn paragraph_midpoints_are_zoom_invariant_and_exclusive_to_paragraph_text() {
    let layer = paragraph_layer(
        Transform {
            x: 80.0,
            y: 60.0,
            rotation: 31.0,
            ..Default::default()
        },
        240.0,
    );
    let source = source_for(&layer);
    let handles = paragraph_width_handle_positions(&layer, Some(source)).unwrap();
    for pixels_per_point in [0.2, 1.0, 8.0] {
        let geometry = CanvasGeometry {
            viewport: Rect::EVERYTHING,
            canvas: Rect::from_min_size(Pos2::new(17.0, 23.0), Vec2::splat(1.0)),
            pixels_per_point,
        };
        assert_eq!(
            resize_handle_at(
                geometry,
                &layer,
                Some(source),
                geometry.canvas_to_screen(handles[0])
            ),
            Some(ResizeHandle::ParagraphLeft)
        );
        assert_eq!(
            resize_handle_at(
                geometry,
                &layer,
                Some(source),
                geometry.canvas_to_screen(handles[1])
            ),
            Some(ResizeHandle::ParagraphRight)
        );
    }

    let mut point_text = layer.clone();
    let LayerKind::Text { typography, .. } = &mut point_text.kind else {
        unreachable!();
    };
    typography.box_width = None;
    assert!(paragraph_width_handle_positions(&point_text, Some(source)).is_none());
    assert!(paragraph_width_handle_positions(&Layer::default(), Some(source)).is_none());
}

#[test]
fn paragraph_cursor_tracks_the_transformed_local_x_axis() {
    for handle in [ResizeHandle::ParagraphLeft, ResizeHandle::ParagraphRight] {
        assert_eq!(
            resize_cursor(handle, 0.0),
            egui::CursorIcon::ResizeHorizontal
        );
        assert_eq!(resize_cursor(handle, 45.0), egui::CursorIcon::ResizeNwSe);
        assert_eq!(
            resize_cursor(handle, 90.0),
            egui::CursorIcon::ResizeVertical
        );
        assert_eq!(resize_cursor(handle, 135.0), egui::CursorIcon::ResizeNeSw);
    }
}

#[test]
fn one_preview_layout_serves_both_outlines_and_the_next_drag() {
    let stale_layer = paragraph_layer(Transform::default(), 180.0);
    let stale_key = LayerVisualKey::new(&stale_layer, 1.0);
    let stale_source = source_for(&stale_layer);

    let mut current = paragraph_layer(Transform::default(), 300.0);
    let current_source =
        current_layer_source_geometry(&current, None, Some((&stale_key, stale_source)), None)
            .unwrap();
    assert_eq!(current_source.paragraph_bounds.unwrap().width(), 300.0);

    let mut drag = width_drag(&current, current_source, ResizeHandle::ParagraphRight);
    drag.current_canvas += Vec2::new(40.0, 0.0);
    let (typography, transform, preview_source) =
        paragraph_width_preview(&current, drag, ResizeHandle::ParagraphRight, None).unwrap();
    let LayerKind::Text {
        typography: current_typography,
        ..
    } = &mut current.kind
    else {
        unreachable!();
    };
    *current_typography = typography;
    current.transform = transform;

    let source_override = LayerSourceOverride::new(current.kind.clone(), preview_source);
    let redundant_layouts = std::cell::Cell::new(0);
    let resolve_again = || {
        redundant_layouts.set(redundant_layouts.get() + 1);
        None
    };
    let outline_source = current_layer_source_geometry_with_resolver(
        &current,
        Some(&source_override),
        Some((&stale_key, stale_source)),
        resolve_again,
    )
    .unwrap();
    let drag_outline_source = current_layer_source_geometry_with_resolver(
        &current,
        Some(&source_override),
        Some((&stale_key, stale_source)),
        resolve_again,
    )
    .unwrap();
    assert_eq!(redundant_layouts.get(), 0);
    assert_eq!(outline_source, preview_source);
    assert_eq!(drag_outline_source, preview_source);

    let latest_source = outline_source;
    assert_eq!(latest_source.paragraph_bounds.unwrap().width(), 340.0);
    let new_drag = width_drag(&current, latest_source, ResizeHandle::ParagraphRight);
    assert_eq!(
        paragraph_width_from_drag(new_drag, ResizeHandle::ParagraphRight),
        Some(340.0)
    );

    let canceled_source = current_layer_source_geometry_with_resolver(
        &stale_layer,
        Some(&source_override),
        Some((&stale_key, stale_source)),
        || None,
    )
    .unwrap();
    assert_eq!(canceled_source, stale_source);
}

#[test]
fn canceling_a_second_width_drag_restores_the_committed_geometry_override() {
    let stale_layer = paragraph_layer(Transform::default(), 180.0);
    let stale_key = LayerVisualKey::new(&stale_layer, 1.0);
    let stale_source = source_for(&stale_layer);
    let committed = paragraph_layer(Transform::default(), 340.0);
    let committed_source = source_for(&committed);
    let preview = paragraph_layer(Transform::default(), 360.0);
    let preview_source = source_for(&preview);

    let mut source_overrides = HashMap::from([(
        committed.id,
        LayerSourceOverride::new(preview.kind.clone(), preview_source),
    )]);
    let mut drag = width_drag(&committed, committed_source, ResizeHandle::ParagraphRight);
    drag.paragraph_source_override = Some(committed_source);
    let mut document = Document::new("Paragraph", 800, 600);
    document.layers.push(committed.clone());

    restore_source_override_after_cancel(&mut source_overrides, &document, drag);
    let redundant_layouts = std::cell::Cell::new(0);
    let restored = current_layer_source_geometry_with_resolver(
        &committed,
        source_overrides.get(&committed.id),
        Some((&stale_key, stale_source)),
        || {
            redundant_layouts.set(redundant_layouts.get() + 1);
            None
        },
    )
    .unwrap();
    assert_eq!(restored, committed_source);
    assert_eq!(redundant_layouts.get(), 0);
}

#[test]
fn unrelated_drag_cancellation_preserves_pending_paragraph_geometry() {
    let paragraph = paragraph_layer(Transform::default(), 340.0);
    let source = source_for(&paragraph);
    let baseline = HashMap::from([(
        paragraph.id,
        LayerSourceOverride::new(paragraph.kind.clone(), source),
    )]);
    let mut document = Document::new("Paragraph", 800, 600);
    document.layers.push(paragraph);

    for action in [DragAction::Move, DragAction::Draw] {
        let mut source_overrides = baseline.clone();
        let drag = DragState {
            action,
            layer_id: Some(7),
            ..width_drag(
                document.layer(7).unwrap(),
                source,
                ResizeHandle::ParagraphRight,
            )
        };
        restore_source_override_after_cancel(&mut source_overrides, &document, drag);
        assert_eq!(source_overrides, baseline);
    }
}

#[test]
fn rotated_width_drag_uses_local_x_and_preserves_typography_and_scale() {
    let layer = paragraph_layer(
        Transform {
            x: 90.0,
            y: 70.0,
            scale_x: 1.75,
            scale_y: 0.8,
            rotation: 37.0,
        },
        240.0,
    );
    let source = source_for(&layer);
    let mut drag = width_drag(&layer, source, ResizeHandle::ParagraphRight);
    drag.current_canvas += rotate_vector(Vec2::new(105.0, 40.0), layer.transform.rotation);
    let (typography, transform, _) =
        paragraph_width_preview(&layer, drag, ResizeHandle::ParagraphRight, None).unwrap();
    let LayerKind::Text {
        font_size,
        typography: before,
        ..
    } = &layer.kind
    else {
        unreachable!();
    };

    assert!((typography.box_width.unwrap() - 300.0).abs() < 0.001);
    assert_eq!(typography.alignment, before.alignment);
    assert_eq!(typography.line_height, before.line_height);
    assert_eq!(typography.tracking, before.tracking);
    assert_eq!(typography.effects, before.effects);
    assert_eq!(*font_size, 32.0);
    assert_eq!(transform.scale_x, layer.transform.scale_x);
    assert_eq!(transform.scale_y, layer.transform.scale_y);
    assert_eq!(transform.rotation, layer.transform.rotation);
}

#[test]
fn width_drag_clamps_and_maps_the_same_canvas_motion_at_every_zoom() {
    let layer = paragraph_layer(Transform::default(), 240.0);
    let source = source_for(&layer);
    let baseline = width_drag(&layer, source, ResizeHandle::ParagraphLeft);
    for pixels_per_point in [0.25, 1.0, 6.0] {
        let geometry = CanvasGeometry {
            viewport: Rect::EVERYTHING,
            canvas: Rect::from_min_size(Pos2::new(13.0, 19.0), Vec2::splat(1.0)),
            pixels_per_point,
        };
        let mut drag = baseline;
        drag.current_canvas = geometry
            .screen_to_canvas(geometry.canvas_to_screen(drag.start_canvas + Vec2::new(-80.0, 0.0)));
        assert!(
            (paragraph_width_from_drag(drag, ResizeHandle::ParagraphLeft).unwrap() - 320.0).abs()
                < 0.001
        );
    }
    let mut clamped = baseline;
    clamped.current_canvas = clamped.start_canvas + Vec2::new(10_000.0, 0.0);
    assert_eq!(
        paragraph_width_from_drag(clamped, ResizeHandle::ParagraphLeft),
        Some(1.0)
    );
}

#[test]
fn left_width_drag_minimally_keeps_the_rotated_opposite_edge_fixed() {
    for rotation in [0.0, 43.0, 90.0] {
        let layer = paragraph_layer(
            Transform {
                x: 120.0,
                y: 95.0,
                scale_x: 1.4,
                scale_y: 0.9,
                rotation,
            },
            260.0,
        );
        let source = source_for(&layer);
        let mut drag = width_drag(&layer, source, ResizeHandle::ParagraphLeft);
        drag.current_canvas += rotate_vector(Vec2::new(-70.0, 0.0), rotation);
        let (_, transform, new_source) =
            paragraph_width_preview(&layer, drag, ResizeHandle::ParagraphLeft, None).unwrap();
        let mut changed = layer.clone();
        changed.transform = transform;
        let old_right = paragraph_width_handle_positions(&layer, Some(source)).unwrap()[1];
        let new_right = paragraph_width_handle_positions(&changed, Some(new_source)).unwrap()[1];
        let normal = rotate_vector(Vec2::new(1.0, 0.0), rotation);
        let tangent = rotate_vector(Vec2::new(0.0, 1.0), rotation);
        assert!(((old_right - new_right).dot(normal)).abs() < 0.001);
        let translation = Vec2::new(
            transform.x - layer.transform.x,
            transform.y - layer.transform.y,
        );
        assert!(translation.dot(tangent).abs() < 0.001);
    }
}

#[test]
fn paragraph_width_preview_is_one_undoable_and_cancelable_revision() {
    let layer = paragraph_layer(Transform::default(), 220.0);
    let source = source_for(&layer);
    let mut drag = width_drag(&layer, source, ResizeHandle::ParagraphRight);
    drag.current_canvas += Vec2::new(80.0, 0.0);
    let (typography, transform, _) =
        paragraph_width_preview(&layer, drag, ResizeHandle::ParagraphRight, None).unwrap();
    let commands = vec![
        Command::SetTextTypography {
            id: layer.id,
            typography,
        },
        Command::SetTransform {
            id: layer.id,
            transform,
        },
    ];
    let mut document = Document::new("Paragraph", 800, 600);
    document.layers.push(layer);
    document.selected = Some(7);
    let mut workspace = Workspace::new(document, None);
    let before = workspace.document.clone();

    workspace.begin_interaction();
    workspace.preview_batch(commands.clone()).unwrap();
    assert!(workspace.commit_interaction().unwrap());
    assert!(workspace.execute(Command::Undo).is_ok());
    assert_eq!(workspace.document, before);
    assert!(workspace.execute(Command::Undo).is_err());

    workspace.begin_interaction();
    workspace.preview_batch(commands).unwrap();
    assert!(workspace.cancel_interaction());
    assert_eq!(workspace.document, before);
}
