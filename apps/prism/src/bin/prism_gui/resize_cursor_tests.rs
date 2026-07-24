use super::*;

fn reference_axis(cursor: egui::CursorIcon) -> Vec2 {
    match cursor {
        egui::CursorIcon::ResizeHorizontal => Vec2::new(1.0, 0.0),
        egui::CursorIcon::ResizeNwSe => Vec2::new(1.0, 1.0).normalized(),
        egui::CursorIcon::ResizeVertical => Vec2::new(0.0, 1.0),
        egui::CursorIcon::ResizeNeSw => Vec2::new(1.0, -1.0).normalized(),
        cursor => panic!("unexpected resize cursor {cursor:?}"),
    }
}

fn rectangle(transform: Transform) -> Layer {
    Layer {
        transform,
        kind: LayerKind::Rectangle {
            width: 160,
            height: 90,
            color: [255; 4],
            corner_radius: 0.0,
        },
        ..Layer::default()
    }
}

fn geometry(pixels_per_point: f32) -> CanvasGeometry {
    CanvasGeometry {
        viewport: Rect::from_min_size(Pos2::ZERO, Vec2::splat(1_000.0)),
        canvas: Rect::from_min_size(Pos2::new(23.0, 47.0), Vec2::splat(1_000.0)),
        pixels_per_point,
    }
}

fn resized_handle_movement(
    geometry: CanvasGeometry,
    layer: &Layer,
    source: LayerSourceGeometry,
    handle: ResizeHandle,
) -> Vec2 {
    let bounds = layer_bounds(layer, Some(source)).expect("layer has visual bounds");
    let corners = rotated_layer_corners(layer, Some(source)).expect("layer has corners");
    let (corner, opposite, signs) = match handle {
        ResizeHandle::TopLeft => (0, 2, Vec2::new(-1.0, -1.0)),
        ResizeHandle::TopRight => (1, 3, Vec2::new(1.0, -1.0)),
        ResizeHandle::BottomRight => (2, 0, Vec2::new(1.0, 1.0)),
        ResizeHandle::BottomLeft => (3, 1, Vec2::new(-1.0, 1.0)),
        ResizeHandle::ParagraphLeft | ResizeHandle::ParagraphRight => {
            panic!("paragraph handles do not use corner resize")
        }
    };
    let anchor = corners[opposite];
    let current_canvas = anchor
        + rotate_vector(
            Vec2::new(bounds.width() * signs.x, bounds.height() * signs.y) * 1.1,
            layer.transform.rotation,
        );
    let resized = drag_transform(
        DragState {
            start_canvas: corners[corner],
            current_canvas,
            layer_id: Some(layer.id),
            transform: layer.transform,
            action: DragAction::Resize(handle),
            bounds: Some(bounds),
            paragraph_bounds: None,
            paragraph_width: None,
            paragraph_source_override: None,
        },
        true,
    );
    let resized_layer = Layer {
        transform: resized,
        ..layer.clone()
    };
    let resized_corner = rotated_layer_corners(&resized_layer, Some(source))
        .expect("resized layer has corners")[corner];
    geometry.canvas_to_screen(resized_corner) - geometry.canvas_to_screen(corners[corner])
}

#[test]
fn corner_resize_cursor_matches_actual_aspect_locked_drag_transform() {
    let source = LayerSourceGeometry::full(Vec2::new(160.0, 90.0));
    let handles = [
        ResizeHandle::TopLeft,
        ResizeHandle::TopRight,
        ResizeHandle::BottomRight,
        ResizeHandle::BottomLeft,
    ];
    let minimum_alignment = (22.5_f32.to_radians()).cos() - 0.001;

    for rotation in [0.0, 90.0, 180.0, 270.0, 31.0, 137.0] {
        for (scale_x, scale_y) in [(1.0, 1.0), (0.5, 2.0), (2.0, 0.5)] {
            let layer = rectangle(Transform {
                x: 240.0,
                y: 180.0,
                scale_x,
                scale_y,
                rotation,
            });
            for pixels_per_point in [0.2, 1.0, 8.0] {
                let geometry = geometry(pixels_per_point);
                for handle in handles {
                    let movement = resized_handle_movement(geometry, &layer, source, handle);
                    let cursor = resize_cursor(geometry, &layer, Some(source), handle)
                        .expect("transformed corner has a resize cursor");
                    assert!(
                        movement.normalized().dot(reference_axis(cursor)).abs()
                            >= minimum_alignment,
                        "rotation={rotation} scale_x={scale_x} scale_y={scale_y} \
                         pixels_per_point={pixels_per_point} {handle:?} chose {cursor:?} \
                         for real drag movement {movement:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn rectangular_arbitrary_rotation_uses_the_opposite_corner_drag_diagonal() {
    let source = LayerSourceGeometry::full(Vec2::new(160.0, 90.0));
    let layer = rectangle(Transform {
        rotation: 31.0,
        ..Transform::default()
    });
    let geometry = geometry(1.0);
    let movement = resized_handle_movement(geometry, &layer, source, ResizeHandle::TopLeft);

    assert_eq!(
        resize_cursor(geometry, &layer, Some(source), ResizeHandle::TopLeft),
        Some(egui::CursorIcon::ResizeNwSe)
    );
    assert!(
        movement
            .normalized()
            .dot(reference_axis(egui::CursorIcon::ResizeNwSe))
            .abs()
            > movement
                .normalized()
                .dot(reference_axis(egui::CursorIcon::ResizeVertical))
                .abs()
    );
}

#[test]
fn cardinal_corner_cursors_rotate_with_the_transformed_handle() {
    let source = LayerSourceGeometry::full(Vec2::new(160.0, 90.0));
    for (rotation, expected) in [
        (0.0, egui::CursorIcon::ResizeNwSe),
        (90.0, egui::CursorIcon::ResizeNeSw),
        (180.0, egui::CursorIcon::ResizeNwSe),
        (270.0, egui::CursorIcon::ResizeNeSw),
    ] {
        assert_eq!(
            resize_cursor(
                geometry(1.0),
                &rectangle(Transform {
                    rotation,
                    ..Transform::default()
                }),
                Some(source),
                ResizeHandle::TopLeft,
            ),
            Some(expected)
        );
    }
}
