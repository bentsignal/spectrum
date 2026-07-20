use super::*;

const RESIZE_HANDLE_HIT_RADIUS: f32 = 18.0;
const RESIZE_HANDLE_SIZE: f32 = 9.0;

#[derive(Clone, Copy)]
pub(super) enum ToggleIcon {
    Visibility,
    Lock,
}

pub(super) fn icon_toggle(ui: &mut egui::Ui, enabled: bool, icon: ToggleIcon) -> egui::Response {
    let response = ui.add_sized([28.0, 26.0], egui::Button::new(""));
    let center = response.rect.center();
    let stroke = Stroke::new(1.5, if enabled { MUTED } else { SUBTLE });
    match icon {
        ToggleIcon::Visibility => {
            let left = center + Vec2::new(-8.0, 0.0);
            let right = center + Vec2::new(8.0, 0.0);
            ui.painter()
                .line_segment([left, center + Vec2::new(0.0, -5.0)], stroke);
            ui.painter()
                .line_segment([center + Vec2::new(0.0, -5.0), right], stroke);
            ui.painter()
                .line_segment([right, center + Vec2::new(0.0, 5.0)], stroke);
            ui.painter()
                .line_segment([center + Vec2::new(0.0, 5.0), left], stroke);
            ui.painter().circle_stroke(center, 2.5, stroke);
            if !enabled {
                ui.painter().line_segment(
                    [center + Vec2::new(-7.0, -7.0), center + Vec2::new(7.0, 7.0)],
                    Stroke::new(1.8, MUTED),
                );
            }
        }
        ToggleIcon::Lock => {
            ui.painter().rect_stroke(
                Rect::from_center_size(center + Vec2::new(0.0, 3.0), Vec2::new(12.0, 9.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            let shackle_left = if enabled { -4.0 } else { -1.0 };
            ui.painter().line_segment(
                [
                    center + Vec2::new(shackle_left, -1.0),
                    center + Vec2::new(shackle_left, -6.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    center + Vec2::new(shackle_left, -6.0),
                    center + Vec2::new(4.0, -6.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [center + Vec2::new(4.0, -6.0), center + Vec2::new(4.0, -1.0)],
                stroke,
            );
        }
    }
    response
}

pub(super) fn canvas_geometry(
    viewport: Rect,
    width: u32,
    height: u32,
    zoom: f32,
    pan: Vec2,
) -> CanvasGeometry {
    let image = Vec2::new(width.max(1) as f32, height.max(1) as f32);
    let fit = (viewport.width() / image.x)
        .min(viewport.height() / image.y)
        .min(1.0);
    let pixels_per_point = fit * zoom;
    let canvas = Rect::from_center_size(viewport.center() + pan, image * pixels_per_point);
    CanvasGeometry {
        viewport,
        canvas,
        pixels_per_point,
    }
}

pub(super) fn layer_bounds(
    layer: &Layer,
    cached_geometry: Option<LayerSourceGeometry>,
) -> Option<Rect> {
    let source = cached_geometry.unwrap_or(match &layer.kind {
        LayerKind::Raster { path, .. } => {
            let (width, height) = image::image_dimensions(path).ok()?;
            LayerSourceGeometry::full(Vec2::new(width as f32, height as f32))
        }
        LayerKind::Text {
            text, font_size, ..
        } => prism_core::measure_text_geometry(text, *font_size)
            .ok()
            .map(|geometry| LayerSourceGeometry {
                size: Vec2::new(geometry.width as f32, geometry.height as f32),
                visual_bounds: Rect::from_min_size(
                    Pos2::new(geometry.visual_left, geometry.visual_top),
                    Vec2::new(geometry.visual_width, geometry.visual_height),
                ),
            })?,
        LayerKind::Rectangle { width, height, .. } => {
            LayerSourceGeometry::full(Vec2::new(*width as f32, *height as f32))
        }
        LayerKind::Ellipse { width, height, .. } => {
            LayerSourceGeometry::full(Vec2::new(*width as f32, *height as f32))
        }
    });
    let min = Pos2::new(
        layer.transform.x + source.visual_bounds.min.x * layer.transform.scale_x,
        layer.transform.y + source.visual_bounds.min.y * layer.transform.scale_y,
    );
    let size = Vec2::new(
        source.visual_bounds.width() * layer.transform.scale_x,
        source.visual_bounds.height() * layer.transform.scale_y,
    );
    Some(Rect::from_min_size(min, size))
}

pub(super) fn rotated_layer_corners(
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
) -> Option<[Pos2; 4]> {
    let bounds = layer_bounds(layer, source_geometry)?;
    let mut corners = [
        bounds.left_top(),
        bounds.right_top(),
        bounds.right_bottom(),
        bounds.left_bottom(),
    ];
    if matches!(layer.kind, LayerKind::Text { .. }) {
        rotate_points_about(&mut corners, bounds.center(), layer.transform.rotation);
    }
    Some(corners)
}

fn rotate_points_about(points: &mut [Pos2], pivot: Pos2, degrees: f32) {
    if degrees.abs() < 0.01 {
        return;
    }
    let (sin, cos) = degrees.to_radians().sin_cos();
    for point in points {
        let delta = *point - pivot;
        *point = pivot + Vec2::new(delta.x * cos - delta.y * sin, delta.x * sin + delta.y * cos);
    }
}

pub(super) fn layer_contains_point(
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
    point: Pos2,
) -> bool {
    let Some(bounds) = layer_bounds(layer, source_geometry) else {
        return false;
    };
    let mut local = [point];
    if matches!(layer.kind, LayerKind::Text { .. }) {
        rotate_points_about(&mut local, bounds.center(), -layer.transform.rotation);
    }
    bounds.contains(local[0])
}

pub(super) fn resize_handle_at(
    geometry: CanvasGeometry,
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
    pointer: Pos2,
) -> Option<ResizeHandle> {
    let corners = rotated_layer_corners(layer, source_geometry)?;
    let corners = [
        (ResizeHandle::TopLeft, corners[0]),
        (ResizeHandle::TopRight, corners[1]),
        (ResizeHandle::BottomLeft, corners[3]),
        (ResizeHandle::BottomRight, corners[2]),
    ];
    corners
        .into_iter()
        .map(|(handle, corner)| (handle, geometry.canvas_to_screen(corner).distance(pointer)))
        .filter(|(_, distance)| *distance <= RESIZE_HANDLE_HIT_RADIUS)
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(handle, _)| handle)
}

pub(super) fn resize_cursor(handle: ResizeHandle) -> egui::CursorIcon {
    match handle {
        ResizeHandle::TopLeft | ResizeHandle::BottomRight => egui::CursorIcon::ResizeNwSe,
        ResizeHandle::TopRight | ResizeHandle::BottomLeft => egui::CursorIcon::ResizeNeSw,
    }
}

pub(super) fn drag_transform(drag: DragState, preserve_aspect: bool) -> Transform {
    if drag.action == DragAction::Move {
        let delta = drag.current_canvas - drag.start_canvas;
        return Transform {
            x: drag.transform.x + delta.x,
            y: drag.transform.y + delta.y,
            ..drag.transform
        };
    }
    let DragAction::Resize(handle) = drag.action else {
        return drag.transform;
    };
    let Some(bounds) = drag.bounds else {
        return drag.transform;
    };
    let geometry_rotation = if drag.visual_rotation_bounds {
        drag.transform.rotation
    } else {
        0.0
    };
    let minimum_width = 1.0_f32.min(bounds.width());
    let minimum_height = 1.0_f32.min(bounds.height());
    let (width, height, opposite) = if geometry_rotation.abs() < 0.01 {
        let width = match handle {
            ResizeHandle::TopLeft | ResizeHandle::BottomLeft => {
                bounds.right() - drag.current_canvas.x
            }
            ResizeHandle::TopRight | ResizeHandle::BottomRight => {
                drag.current_canvas.x - bounds.left()
            }
        };
        let height = match handle {
            ResizeHandle::TopLeft | ResizeHandle::TopRight => {
                bounds.bottom() - drag.current_canvas.y
            }
            ResizeHandle::BottomLeft | ResizeHandle::BottomRight => {
                drag.current_canvas.y - bounds.top()
            }
        };
        let opposite = match handle {
            ResizeHandle::TopLeft => bounds.right_bottom(),
            ResizeHandle::TopRight => bounds.left_bottom(),
            ResizeHandle::BottomLeft => bounds.right_top(),
            ResizeHandle::BottomRight => bounds.left_top(),
        };
        (width, height, opposite)
    } else {
        let corners = rotated_rect_corners(bounds, geometry_rotation);
        let opposite = match handle {
            ResizeHandle::TopLeft => corners[2],
            ResizeHandle::TopRight => corners[3],
            ResizeHandle::BottomLeft => corners[1],
            ResizeHandle::BottomRight => corners[0],
        };
        let local = rotate_vector(drag.current_canvas - opposite, -geometry_rotation);
        let width = match handle {
            ResizeHandle::TopLeft | ResizeHandle::BottomLeft => -local.x,
            ResizeHandle::TopRight | ResizeHandle::BottomRight => local.x,
        };
        let height = match handle {
            ResizeHandle::TopLeft | ResizeHandle::TopRight => -local.y,
            ResizeHandle::BottomLeft | ResizeHandle::BottomRight => local.y,
        };
        (width, height, opposite)
    };
    let width = width.max(minimum_width);
    let height = height.max(minimum_height);
    let mut ratio_x = width / bounds.width().max(0.001);
    let mut ratio_y = height / bounds.height().max(0.001);
    if preserve_aspect {
        let ratio = ((ratio_x + ratio_y) * 0.5).max(0.01);
        ratio_x = ratio;
        ratio_y = ratio;
    }
    let width = bounds.width() * ratio_x;
    let height = bounds.height() * ratio_y;
    let local_center_from_opposite = match handle {
        ResizeHandle::TopLeft => Vec2::new(-width * 0.5, -height * 0.5),
        ResizeHandle::TopRight => Vec2::new(width * 0.5, -height * 0.5),
        ResizeHandle::BottomLeft => Vec2::new(-width * 0.5, height * 0.5),
        ResizeHandle::BottomRight => Vec2::new(width * 0.5, height * 0.5),
    };
    let center = opposite + rotate_vector(local_center_from_opposite, geometry_rotation);
    let visual_offset_x = (bounds.left() - drag.transform.x) * ratio_x;
    let visual_offset_y = (bounds.top() - drag.transform.y) * ratio_y;
    let x = center.x - width * 0.5 - visual_offset_x;
    let y = center.y - height * 0.5 - visual_offset_y;
    Transform {
        x,
        y,
        scale_x: (drag.transform.scale_x * ratio_x).max(0.01),
        scale_y: (drag.transform.scale_y * ratio_y).max(0.01),
        rotation: drag.transform.rotation,
    }
}

fn rotated_rect_corners(bounds: Rect, degrees: f32) -> [Pos2; 4] {
    let mut corners = [
        bounds.left_top(),
        bounds.right_top(),
        bounds.right_bottom(),
        bounds.left_bottom(),
    ];
    rotate_points_about(&mut corners, bounds.center(), degrees);
    corners
}

fn rotate_vector(vector: Vec2, degrees: f32) -> Vec2 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    Vec2::new(
        vector.x * cos - vector.y * sin,
        vector.x * sin + vector.y * cos,
    )
}

pub(super) fn drag_rotation(drag: DragState, snap: bool) -> f32 {
    let Some(bounds) = drag.bounds else {
        return drag.transform.rotation;
    };
    let center = bounds.center();
    let start = drag.start_canvas - center;
    let current = drag.current_canvas - center;
    if start.length_sq() < 0.001 || current.length_sq() < 0.001 {
        return drag.transform.rotation;
    }
    let delta = current.y.atan2(current.x) - start.y.atan2(start.x);
    let degrees = drag.transform.rotation + delta.to_degrees();
    if snap {
        ((degrees / 15.0).round() * 15.0).rem_euclid(360.0)
    } else {
        degrees.rem_euclid(360.0)
    }
}

pub(super) fn paint_layer_outline(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
    offset: Vec2,
) {
    paint_selection_outline(ui, geometry, layer, source_geometry, offset, true);
}

pub(super) fn paint_rotation_outline(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
) {
    paint_selection_outline(ui, geometry, layer, source_geometry, Vec2::ZERO, false);
}

fn paint_selection_outline(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
    offset: Vec2,
    show_resize_handles: bool,
) {
    if !matches!(layer.kind, LayerKind::Text { .. }) {
        let Some(bounds) = layer_bounds(layer, source_geometry) else {
            return;
        };
        let rect = Rect::from_min_max(
            geometry.canvas_to_screen(bounds.min) + offset,
            geometry.canvas_to_screen(bounds.max) + offset,
        );
        ui.painter().with_clip_rect(geometry.viewport).rect_stroke(
            rect,
            1.0,
            Stroke::new(1.5, ACCENT),
            egui::StrokeKind::Outside,
        );
        if show_resize_handles {
            for corner in [
                rect.left_top(),
                rect.right_top(),
                rect.left_bottom(),
                rect.right_bottom(),
            ] {
                paint_resize_handle(ui, corner);
            }
        }
        return;
    }
    let Some(corners) = rotated_layer_corners(layer, source_geometry) else {
        return;
    };
    let corners = corners.map(|corner| geometry.canvas_to_screen(corner) + offset);
    ui.painter()
        .with_clip_rect(geometry.viewport)
        .add(egui::Shape::closed_line(
            corners.to_vec(),
            Stroke::new(1.5, ACCENT),
        ));
    if show_resize_handles {
        for corner in corners {
            paint_resize_handle(ui, corner);
        }
    }
}

fn paint_resize_handle(ui: &egui::Ui, corner: Pos2) {
    ui.painter().rect_filled(
        Rect::from_center_size(corner, Vec2::splat(RESIZE_HANDLE_SIZE)),
        1.0,
        ACCENT,
    );
}

pub(super) fn contrast_text(background: Color32) -> Color32 {
    let luma = background.r() as u16 * 3 + background.g() as u16 * 6 + background.b() as u16;
    if luma > 1_400 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

pub(super) fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(contrast_text(ACCENT)))
            .fill(ACCENT)
            .stroke(Stroke::new(1.0, ACCENT)),
    )
}

pub(super) fn quiet_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(label).size(11.0)).frame(false))
}

pub(super) fn quiet_danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(label).size(11.0).color(DANGER)).frame(false))
}

pub(super) fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(contrast_text(DANGER)))
            .fill(DANGER)
            .stroke(Stroke::new(1.0, DANGER)),
    )
}

pub(super) fn color32(value: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(value[0], value[1], value[2], value[3])
}

pub(super) fn rgba(value: Color32) -> [u8; 4] {
    [value.r(), value.g(), value.b(), value.a()]
}

pub(super) fn with_alpha(value: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(value.r(), value.g(), value.b(), alpha)
}

fn paint_shortcut_key_background(ui: &egui::Ui, rect: Rect) {
    ui.painter().rect(
        rect,
        2.0,
        WORKSPACE,
        Stroke::new(1.0, BORDER),
        egui::StrokeKind::Inside,
    );
}

fn paint_shortcut_key(ui: &egui::Ui, rect: Rect, key: &str) {
    paint_shortcut_key_background(ui, rect);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        key,
        FontId::monospace(9.5),
        SUBTLE,
    );
}

fn paint_option_key(ui: &egui::Ui, rect: Rect) {
    paint_shortcut_key_background(ui, rect);
    let left = rect.left() + 5.5;
    let right = rect.right() - 5.5;
    let top = rect.top() + 6.4;
    let bottom = rect.bottom() - 6.4;
    let bend = rect.center().x - 0.8;
    let stroke = Stroke::new(1.35, MUTED);
    ui.painter()
        .line_segment([Pos2::new(left, top), Pos2::new(bend, top)], stroke);
    ui.painter().line_segment(
        [Pos2::new(bend, top), Pos2::new(bend + 3.6, bottom)],
        stroke,
    );
    ui.painter().line_segment(
        [Pos2::new(bend + 3.6, bottom), Pos2::new(right, bottom)],
        stroke,
    );
    ui.painter()
        .line_segment([Pos2::new(bend + 2.7, top), Pos2::new(right, top)], stroke);
}

pub(super) fn shortcut_key(ui: &mut egui::Ui, key: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
    paint_shortcut_key(ui, rect, key);
    response
}

fn paint_modified_shortcut(
    ui: &egui::Ui,
    rect: Rect,
    modifier: &str,
    modifier_width: f32,
    key: &str,
) {
    let modifier_rect = Rect::from_min_size(rect.min, Vec2::new(modifier_width, 20.0));
    let key_rect = Rect::from_min_size(
        Pos2::new(modifier_rect.right() + 3.0, rect.top()),
        Vec2::splat(20.0),
    );
    paint_shortcut_key(ui, modifier_rect, modifier);
    paint_shortcut_key(ui, key_rect, key);
}

pub(super) fn paint_command_shortcut(ui: &egui::Ui, rect: Rect, key: &str) {
    paint_modified_shortcut(ui, rect, "⌘", 20.0, key);
}

pub(super) fn command_shortcut(ui: &mut egui::Ui, key: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(43.0, 20.0), Sense::hover());
    paint_command_shortcut(ui, rect, key);
    response
}

pub(super) fn alternate_shortcut(ui: &mut egui::Ui, key: &str) -> egui::Response {
    if cfg!(target_os = "macos") {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(43.0, 20.0), Sense::hover());
        let modifier_rect = Rect::from_min_size(rect.min, Vec2::splat(20.0));
        let key_rect = Rect::from_min_size(
            Pos2::new(modifier_rect.right() + 3.0, rect.top()),
            Vec2::splat(20.0),
        );
        paint_option_key(ui, modifier_rect);
        paint_shortcut_key(ui, key_rect, key);
        response
    } else {
        let modifier_width = 28.0;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(modifier_width + 23.0, 20.0), Sense::hover());
        paint_modified_shortcut(ui, rect, "Alt", modifier_width, key);
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_layer(transform: Transform) -> Layer {
        Layer {
            transform,
            kind: LayerKind::Text {
                text: "Rotate".into(),
                font_size: 32.0,
                color: [255, 255, 255, 255],
            },
            ..Layer::default()
        }
    }

    fn rotation_drag(current_canvas: Pos2) -> DragState {
        DragState {
            start_canvas: Pos2::new(100.0, 50.0),
            current_canvas,
            layer_id: Some(1),
            transform: Transform {
                rotation: 10.0,
                ..Default::default()
            },
            action: DragAction::Rotate,
            bounds: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 100.0))),
            visual_rotation_bounds: false,
        }
    }

    #[test]
    fn rotation_drag_tracks_clockwise_canvas_angle() {
        assert!(
            (drag_rotation(rotation_drag(Pos2::new(50.0, 100.0)), false) - 100.0).abs() < 0.001
        );
    }

    #[test]
    fn rotation_drag_snaps_absolute_angle_to_fifteen_degrees() {
        assert_eq!(
            drag_rotation(rotation_drag(Pos2::new(50.0, 100.0)), true),
            105.0
        );
    }

    #[test]
    fn rotation_drag_snaps_cleanly_across_zero_degrees() {
        let mut drag = rotation_drag(Pos2::new(99.240_39, 58.682_41));
        drag.transform.rotation = 355.0;
        assert_eq!(drag_rotation(drag, true), 0.0);
    }

    #[test]
    fn rotated_visual_bounds_keep_the_same_center_at_varied_zoom() {
        let layer = text_layer(Transform {
            x: 80.0,
            y: 50.0,
            scale_x: 1.5,
            scale_y: 0.75,
            rotation: 37.0,
        });
        let source = LayerSourceGeometry {
            size: Vec2::new(140.0, 80.0),
            visual_bounds: Rect::from_min_size(Pos2::new(12.0, 18.0), Vec2::new(96.0, 42.0)),
        };
        let bounds = layer_bounds(&layer, Some(source)).unwrap();
        let corners = rotated_layer_corners(&layer, Some(source)).unwrap();
        let canvas_center = corners
            .iter()
            .fold(Vec2::ZERO, |sum, point| sum + point.to_vec2())
            / 4.0;
        assert!(Pos2::new(canvas_center.x, canvas_center.y).distance(bounds.center()) < 0.001);

        for pixels_per_point in [0.2, 1.0, 8.0] {
            let geometry = CanvasGeometry {
                viewport: Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0)),
                canvas: Rect::from_min_size(Pos2::new(17.0, 23.0), Vec2::splat(1.0)),
                pixels_per_point,
            };
            let screen_center = corners.iter().fold(Vec2::ZERO, |sum, point| {
                sum + geometry.canvas_to_screen(*point).to_vec2()
            }) / 4.0;
            assert!(
                Pos2::new(screen_center.x, screen_center.y)
                    .distance(geometry.canvas_to_screen(bounds.center()))
                    < 0.001
            );
        }
    }

    #[test]
    fn hit_testing_follows_rotated_visual_bounds() {
        let layer = text_layer(Transform {
            rotation: 90.0,
            ..Transform::default()
        });
        let source = LayerSourceGeometry::full(Vec2::new(100.0, 20.0));

        assert!(layer_contains_point(
            &layer,
            Some(source),
            Pos2::new(50.0, -20.0)
        ));
        assert!(!layer_contains_point(
            &layer,
            Some(source),
            Pos2::new(10.0, 10.0)
        ));
    }

    #[test]
    fn unrotated_shape_bounds_preserve_the_existing_top_left_semantics() {
        let layer = Layer {
            transform: Transform {
                x: 20.0,
                y: 30.0,
                scale_x: 2.0,
                scale_y: 3.0,
                ..Transform::default()
            },
            ..Layer::default()
        };
        let bounds = layer_bounds(
            &layer,
            Some(LayerSourceGeometry::full(Vec2::new(100.0, 80.0))),
        )
        .unwrap();

        assert_eq!(bounds.min, Pos2::new(20.0, 30.0));
        assert_eq!(bounds.size(), Vec2::new(200.0, 240.0));
    }

    #[test]
    fn shape_selection_geometry_keeps_its_existing_rotation_behavior() {
        let layer = Layer {
            transform: Transform {
                x: 20.0,
                y: 30.0,
                rotation: 90.0,
                ..Transform::default()
            },
            ..Layer::default()
        };
        let corners = rotated_layer_corners(
            &layer,
            Some(LayerSourceGeometry::full(Vec2::new(100.0, 80.0))),
        )
        .unwrap();

        assert_eq!(corners[0], Pos2::new(20.0, 30.0));
        assert_eq!(corners[2], Pos2::new(120.0, 110.0));
    }

    #[test]
    fn shape_resize_keeps_its_existing_axis_aligned_behavior_when_rotated() {
        let transform = drag_transform(
            DragState {
                start_canvas: Pos2::new(110.0, 70.0),
                current_canvas: Pos2::new(210.0, 120.0),
                layer_id: Some(1),
                transform: Transform {
                    x: 10.0,
                    y: 20.0,
                    rotation: 90.0,
                    ..Transform::default()
                },
                action: DragAction::Resize(ResizeHandle::BottomRight),
                bounds: Some(Rect::from_min_size(
                    Pos2::new(10.0, 20.0),
                    Vec2::new(100.0, 50.0),
                )),
                visual_rotation_bounds: false,
            },
            true,
        );

        assert_eq!((transform.x, transform.y), (10.0, 20.0));
        assert_eq!((transform.scale_x, transform.scale_y), (2.0, 2.0));
        assert_eq!(transform.rotation, 90.0);
    }

    #[test]
    fn rotated_text_resize_keeps_the_opposite_visual_corner_fixed() {
        let original_opposite = Pos2::new(60.0, -40.0);
        let transform = drag_transform(
            DragState {
                start_canvas: Pos2::new(40.0, 60.0),
                current_canvas: Pos2::new(20.0, 160.0),
                layer_id: Some(1),
                transform: Transform {
                    rotation: 90.0,
                    ..Transform::default()
                },
                action: DragAction::Resize(ResizeHandle::BottomRight),
                bounds: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 20.0))),
                visual_rotation_bounds: true,
            },
            false,
        );
        let resized_bounds =
            Rect::from_min_size(Pos2::new(transform.x, transform.y), Vec2::new(200.0, 40.0));
        let resized_corners = rotated_rect_corners(resized_bounds, transform.rotation);

        assert!(resized_corners[0].distance(original_opposite) < 0.001);
        assert!((transform.scale_x - 2.0).abs() < 0.001);
        assert!((transform.scale_y - 2.0).abs() < 0.001);
    }
}
