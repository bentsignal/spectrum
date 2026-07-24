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

#[test]
fn corner_resize_cursor_tracks_screen_space_drag_axis_at_rotation_zoom_and_mirroring() {
    let source = LayerSourceGeometry::full(Vec2::new(160.0, 90.0));
    let handles = [
        ResizeHandle::TopLeft,
        ResizeHandle::TopRight,
        ResizeHandle::BottomRight,
        ResizeHandle::BottomLeft,
    ];
    let minimum_alignment = (22.5_f32.to_radians()).cos() - 0.001;

    for rotation in [0.0, 90.0, 180.0, 270.0, 31.0, 137.0] {
        for (scale_x, scale_y) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
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
                    let axis = resize_screen_axis(geometry, &layer, Some(source), handle)
                        .expect("transformed corner has a resize axis");
                    let cursor = resize_cursor(geometry, &layer, Some(source), handle)
                        .expect("transformed corner has a resize cursor");
                    assert!(
                        axis.dot(reference_axis(cursor)).abs() >= minimum_alignment,
                        "rotation={rotation} scale_x={scale_x} scale_y={scale_y} \
                         pixels_per_point={pixels_per_point} {handle:?} chose {cursor:?} \
                         for axis {axis:?}"
                    );
                }
            }
        }
    }
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
