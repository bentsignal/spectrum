use super::*;

#[path = "controls.rs"]
mod controls;
pub(super) use controls::*;

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
    let source = resolved_source_geometry(layer, cached_geometry)?;
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

fn resolved_source_geometry(
    layer: &Layer,
    cached_geometry: Option<LayerSourceGeometry>,
) -> Option<LayerSourceGeometry> {
    cached_geometry.or_else(|| match &layer.kind {
        LayerKind::Raster { path, .. } => {
            let (width, height) = image::image_dimensions(path).ok()?;
            Some(LayerSourceGeometry::full(Vec2::new(
                width as f32,
                height as f32,
            )))
        }
        LayerKind::Text {
            text,
            font_size,
            typography,
            ..
        } => prism_core::measure_text_geometry_with_typography(text, *font_size, typography, None)
            .ok()
            .map(|geometry| text_source_geometry(geometry, typography.box_width.is_some())),
        LayerKind::Rectangle { width, height, .. } => Some(LayerSourceGeometry::full(Vec2::new(
            *width as f32,
            *height as f32,
        ))),
        LayerKind::Ellipse { width, height, .. } => Some(LayerSourceGeometry::full(Vec2::new(
            *width as f32,
            *height as f32,
        ))),
        LayerKind::Path { geometry, .. } => {
            prism_core::path_source_bounds(layer).map(|bounds| LayerSourceGeometry {
                size: Vec2::new(geometry.width() as f32, geometry.height() as f32),
                visual_bounds: Rect::from_min_size(
                    Pos2::new(bounds.origin[0], bounds.origin[1]),
                    Vec2::new(bounds.size[0], bounds.size[1]),
                ),
                paragraph_bounds: None,
            })
        }
        LayerKind::Paint { program } => Some(LayerSourceGeometry::full(Vec2::new(
            program.width as f32,
            program.height as f32,
        ))),
    })
}

fn transformed_layer_geometry(
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
) -> Option<prism_core::LayerGeometry> {
    let source = resolved_source_geometry(layer, source_geometry)?;
    Some(prism_core::layer_geometry_with_bounds(
        layer,
        [source.visual_bounds.left(), source.visual_bounds.top()],
        [source.visual_bounds.width(), source.visual_bounds.height()],
    ))
}

pub(super) fn rotated_layer_corners(
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
) -> Option<[Pos2; 4]> {
    Some(
        transformed_layer_geometry(layer, source_geometry)?
            .corners
            .map(|corner| Pos2::new(corner[0], corner[1])),
    )
}

pub(super) fn layer_contains_point(
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
    point: Pos2,
) -> bool {
    let Some(bounds) = layer_bounds(layer, source_geometry) else {
        return false;
    };
    let unrotated =
        bounds.center() + rotate_vector(point - bounds.center(), -layer.transform.rotation);
    bounds.contains(unrotated)
}

pub(super) fn resize_handle_at(
    geometry: CanvasGeometry,
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
    pointer: Pos2,
) -> Option<ResizeHandle> {
    let corners = rotated_layer_corners(layer, source_geometry)?;
    let mut handles = vec![
        (ResizeHandle::TopLeft, corners[0]),
        (ResizeHandle::TopRight, corners[1]),
        (ResizeHandle::BottomRight, corners[2]),
        (ResizeHandle::BottomLeft, corners[3]),
    ];
    if let Some([left, right]) = paragraph_width_handle_positions(layer, source_geometry) {
        handles.extend([
            (ResizeHandle::ParagraphLeft, left),
            (ResizeHandle::ParagraphRight, right),
        ]);
    }
    handles
        .into_iter()
        .map(|(handle, corner)| (handle, geometry.canvas_to_screen(corner).distance(pointer)))
        .filter(|(_, distance)| *distance <= RESIZE_HANDLE_HIT_RADIUS)
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(handle, _)| handle)
}

pub(super) fn resize_cursor(
    geometry: CanvasGeometry,
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
    handle: ResizeHandle,
) -> Option<egui::CursorIcon> {
    resize_screen_axis(geometry, layer, source_geometry, handle).map(cursor_for_screen_axis)
}

pub(super) fn resize_screen_axis(
    geometry: CanvasGeometry,
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
    handle: ResizeHandle,
) -> Option<Vec2> {
    if matches!(
        handle,
        ResizeHandle::ParagraphLeft | ResizeHandle::ParagraphRight
    ) {
        let [left, right] = paragraph_width_handle_positions(layer, source_geometry)?;
        return nonzero_axis(geometry.canvas_to_screen(right) - geometry.canvas_to_screen(left));
    }

    let corners = rotated_layer_corners(layer, source_geometry)?
        .map(|point| geometry.canvas_to_screen(point));
    let (corner, first_neighbor, second_neighbor) = match handle {
        ResizeHandle::TopLeft => (0, 1, 3),
        ResizeHandle::TopRight => (1, 0, 2),
        ResizeHandle::BottomRight => (2, 1, 3),
        ResizeHandle::BottomLeft => (3, 0, 2),
        ResizeHandle::ParagraphLeft | ResizeHandle::ParagraphRight => unreachable!(),
    };
    let first = nonzero_axis(corners[corner] - corners[first_neighbor])?;
    let second = nonzero_axis(corners[corner] - corners[second_neighbor])?;
    nonzero_axis(first + second)
}

fn nonzero_axis(axis: Vec2) -> Option<Vec2> {
    (axis.length_sq() > f32::EPSILON).then(|| axis.normalized())
}

fn cursor_for_screen_axis(axis: Vec2) -> egui::CursorIcon {
    let angle = axis.angle().to_degrees().rem_euclid(180.0);
    match (((angle + 22.5) / 45.0).floor() as u32) % 4 {
        0 => egui::CursorIcon::ResizeHorizontal,
        1 => egui::CursorIcon::ResizeNwSe,
        2 => egui::CursorIcon::ResizeVertical,
        _ => egui::CursorIcon::ResizeNeSw,
    }
}

pub(super) fn paragraph_width_handle_positions(
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
) -> Option<[Pos2; 2]> {
    let LayerKind::Text { typography, .. } = &layer.kind else {
        return None;
    };
    typography.box_width?;
    let source = source_geometry?;
    let paragraph = source.paragraph_bounds?;
    let visual = layer_bounds(layer, Some(source))?;
    let paragraph = transformed_source_bounds(layer.transform, paragraph);
    Some([
        rotated_point(
            paragraph.left_center(),
            visual.center(),
            layer.transform.rotation,
        ),
        rotated_point(
            paragraph.right_center(),
            visual.center(),
            layer.transform.rotation,
        ),
    ])
}

pub(super) fn paragraph_layer_bounds(
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
) -> Option<Rect> {
    let LayerKind::Text { typography, .. } = &layer.kind else {
        return None;
    };
    typography.box_width?;
    let source = source_geometry?;
    Some(transformed_source_bounds(
        layer.transform,
        source.paragraph_bounds?,
    ))
}

pub(super) fn paragraph_box_width(layer: &Layer) -> Option<f32> {
    let LayerKind::Text { typography, .. } = &layer.kind else {
        return None;
    };
    typography.box_width
}

fn transformed_source_bounds(transform: Transform, source: Rect) -> Rect {
    Rect::from_min_size(
        Pos2::new(
            transform.x + source.left() * transform.scale_x,
            transform.y + source.top() * transform.scale_y,
        ),
        Vec2::new(
            source.width() * transform.scale_x,
            source.height() * transform.scale_y,
        ),
    )
}

pub(super) fn paragraph_width_from_drag(drag: DragState, handle: ResizeHandle) -> Option<f32> {
    let width = drag.paragraph_width?;
    let direction = match handle {
        ResizeHandle::ParagraphLeft => -1.0,
        ResizeHandle::ParagraphRight => 1.0,
        _ => return None,
    };
    let local_delta = rotate_vector(
        drag.current_canvas - drag.start_canvas,
        -drag.transform.rotation,
    );
    let scale_x = drag.transform.scale_x.max(0.01);
    Some((width + direction * local_delta.x / scale_x).clamp(1.0, 100_000.0))
}

pub(super) fn anchored_paragraph_transform(
    drag: DragState,
    handle: ResizeHandle,
    new_layer: &Layer,
    new_source: LayerSourceGeometry,
) -> Option<Transform> {
    let old_visual = drag.bounds?;
    let old_paragraph = drag.paragraph_bounds?;
    let new_visual = layer_bounds(new_layer, Some(new_source))?;
    let new_paragraph =
        transformed_source_bounds(new_layer.transform, new_source.paragraph_bounds?);
    let (old_anchor, new_anchor) = match handle {
        ResizeHandle::ParagraphLeft => (old_paragraph.right_center(), new_paragraph.right_center()),
        ResizeHandle::ParagraphRight => (old_paragraph.left_center(), new_paragraph.left_center()),
        _ => return None,
    };
    let old_anchor = rotated_point(old_anchor, old_visual.center(), drag.transform.rotation);
    let new_anchor = rotated_point(
        new_anchor,
        new_visual.center(),
        new_layer.transform.rotation,
    );
    let normal = rotate_vector(Vec2::new(1.0, 0.0), drag.transform.rotation);
    let correction = normal * (old_anchor - new_anchor).dot(normal);
    Some(Transform {
        x: drag.transform.x + correction.x,
        y: drag.transform.y + correction.y,
        ..drag.transform
    })
}

pub(super) fn paragraph_width_preview(
    layer: &Layer,
    drag: DragState,
    handle: ResizeHandle,
    font_asset: Option<&prism_core::FontAsset>,
) -> Option<(prism_core::TextTypography, Transform, LayerSourceGeometry)> {
    let width = paragraph_width_from_drag(drag, handle)?;
    let mut candidate = layer.clone();
    candidate.transform = drag.transform;
    let LayerKind::Text {
        text,
        font_size,
        typography,
        ..
    } = &mut candidate.kind
    else {
        return None;
    };
    typography.box_width = Some(width);
    let geometry =
        prism_core::measure_text_geometry_with_typography(text, *font_size, typography, font_asset)
            .ok()?;
    let source = text_source_geometry(geometry, true);
    let typography = typography.clone();
    let transform = anchored_paragraph_transform(drag, handle, &candidate, source)?;
    Some((typography, transform, source))
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
    if matches!(
        handle,
        ResizeHandle::ParagraphLeft | ResizeHandle::ParagraphRight
    ) {
        return drag.transform;
    }
    let Some(bounds) = drag.bounds else {
        return drag.transform;
    };
    let minimum_width = 1.0_f32.min(bounds.width());
    let minimum_height = 1.0_f32.min(bounds.height());
    let anchor = match handle {
        ResizeHandle::TopLeft => rotated_point(
            bounds.right_bottom(),
            bounds.center(),
            drag.transform.rotation,
        ),
        ResizeHandle::TopRight => rotated_point(
            bounds.left_bottom(),
            bounds.center(),
            drag.transform.rotation,
        ),
        ResizeHandle::BottomLeft => {
            rotated_point(bounds.right_top(), bounds.center(), drag.transform.rotation)
        }
        ResizeHandle::BottomRight => {
            rotated_point(bounds.left_top(), bounds.center(), drag.transform.rotation)
        }
        ResizeHandle::ParagraphLeft | ResizeHandle::ParagraphRight => return drag.transform,
    };
    let local_delta = rotate_vector(drag.current_canvas - anchor, -drag.transform.rotation);
    let signs = match handle {
        ResizeHandle::TopLeft => Vec2::new(-1.0, -1.0),
        ResizeHandle::TopRight => Vec2::new(1.0, -1.0),
        ResizeHandle::BottomLeft => Vec2::new(-1.0, 1.0),
        ResizeHandle::BottomRight => Vec2::new(1.0, 1.0),
        ResizeHandle::ParagraphLeft | ResizeHandle::ParagraphRight => return drag.transform,
    };
    let width = (local_delta.x * signs.x).max(minimum_width);
    let height = (local_delta.y * signs.y).max(minimum_height);
    let mut ratio_x = width / bounds.width().max(0.001);
    let mut ratio_y = height / bounds.height().max(0.001);
    if preserve_aspect {
        let ratio = ((ratio_x + ratio_y) * 0.5).max(0.01);
        ratio_x = ratio;
        ratio_y = ratio;
    }
    let width = bounds.width() * ratio_x;
    let height = bounds.height() * ratio_y;
    let center = anchor
        + rotate_vector(
            Vec2::new(width * signs.x, height * signs.y) * 0.5,
            drag.transform.rotation,
        );
    let visual_offset_x = (bounds.left() - drag.transform.x) * ratio_x;
    let visual_offset_y = (bounds.top() - drag.transform.y) * ratio_y;
    Transform {
        x: center.x - width * 0.5 - visual_offset_x,
        y: center.y - height * 0.5 - visual_offset_y,
        scale_x: (drag.transform.scale_x * ratio_x).max(0.01),
        scale_y: (drag.transform.scale_y * ratio_y).max(0.01),
        rotation: drag.transform.rotation,
    }
}

pub(super) fn rotate_vector(vector: Vec2, degrees: f32) -> Vec2 {
    let (sin, cos) = prism_core::rotation_sin_cos(degrees);
    Vec2::new(
        vector.x * cos - vector.y * sin,
        vector.x * sin + vector.y * cos,
    )
}

fn rotated_point(point: Pos2, center: Pos2, degrees: f32) -> Pos2 {
    center + rotate_vector(point - center, degrees)
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
    let Some(layer_geometry) = transformed_layer_geometry(layer, source_geometry) else {
        return;
    };
    let corners = layer_geometry
        .corners
        .map(|corner| geometry.canvas_to_screen(Pos2::new(corner[0], corner[1])) + offset);
    let painter = ui.painter().with_clip_rect(geometry.viewport);
    for index in 0..corners.len() {
        painter.line_segment(
            [corners[index], corners[(index + 1) % corners.len()]],
            Stroke::new(1.5, ACCENT),
        );
    }
    if show_resize_handles {
        for corner in corners {
            paint_resize_handle(ui, corner);
        }
        if let Some(handles) = paragraph_width_handle_positions(layer, source_geometry) {
            for handle in handles {
                paint_resize_handle(ui, geometry.canvas_to_screen(handle) + offset);
            }
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
    if cfg!(target_os = "macos") {
        paint_modified_shortcut(ui, rect, "⌘", 20.0, key);
    } else {
        paint_modified_shortcut(ui, rect, "Ctrl", 28.0, key);
    }
}

pub(super) fn command_shortcut(ui: &mut egui::Ui, key: &str) -> egui::Response {
    let width = if cfg!(target_os = "macos") {
        43.0
    } else {
        51.0
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 20.0), Sense::hover());
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
#[path = "resize_cursor_tests.rs"]
mod resize_cursor_tests;

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
                typography: prism_core::TextTypography::default(),
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
            paragraph_bounds: None,
            paragraph_width: None,
            paragraph_source_override: None,
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
            paragraph_bounds: None,
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
    fn rotated_shape_selection_uses_the_transformed_quad() {
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

        assert!(corners[0].distance(Pos2::new(110.0, 20.0)) < 0.001);
        assert!(corners[2].distance(Pos2::new(30.0, 120.0)) < 0.001);
    }

    #[test]
    fn hit_testing_follows_the_rotated_quad_instead_of_its_source_rect() {
        let layer = Layer {
            transform: Transform {
                x: 100.0,
                y: 100.0,
                rotation: 90.0,
                ..Default::default()
            },
            kind: LayerKind::Rectangle {
                width: 100,
                height: 40,
                color: [255; 4],
                corner_radius: 0.0,
            },
            ..Default::default()
        };
        let source = Some(LayerSourceGeometry::full(Vec2::new(100.0, 40.0)));
        assert!(layer_contains_point(
            &layer,
            source,
            Pos2::new(150.0, 160.0)
        ));
        assert!(!layer_contains_point(
            &layer,
            source,
            Pos2::new(195.0, 120.0)
        ));
    }

    #[test]
    fn rotated_resize_keeps_the_opposite_transformed_corner_fixed() {
        let bounds = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(100.0, 50.0));
        let anchor = rotated_point(bounds.left_top(), bounds.center(), 90.0);
        let drag = DragState {
            start_canvas: rotated_point(bounds.right_bottom(), bounds.center(), 90.0),
            current_canvas: anchor + rotate_vector(Vec2::new(120.0, 60.0), 90.0),
            layer_id: Some(1),
            transform: Transform {
                x: 100.0,
                y: 100.0,
                rotation: 90.0,
                ..Default::default()
            },
            action: DragAction::Resize(ResizeHandle::BottomRight),
            bounds: Some(bounds),
            paragraph_bounds: None,
            paragraph_width: None,
            paragraph_source_override: None,
        };
        let resized = drag_transform(drag, false);
        assert!((resized.scale_x - 1.2).abs() < 0.001);
        assert!((resized.scale_y - 1.2).abs() < 0.001);
        let resized_bounds =
            Rect::from_min_size(Pos2::new(resized.x, resized.y), Vec2::new(120.0, 60.0));
        let resized_anchor = rotated_point(
            resized_bounds.left_top(),
            resized_bounds.center(),
            resized.rotation,
        );
        assert!(resized_anchor.distance(anchor) < 0.001);
    }

    #[test]
    fn rotated_text_resize_scales_visible_offset_and_keeps_opposite_corner_fixed() {
        let source = LayerSourceGeometry {
            size: Vec2::new(140.0, 80.0),
            visual_bounds: Rect::from_min_size(Pos2::new(12.0, 18.0), Vec2::new(96.0, 42.0)),
            paragraph_bounds: None,
        };
        let transform = Transform {
            rotation: 90.0,
            ..Transform::default()
        };
        let bounds = layer_bounds(&text_layer(transform), Some(source)).unwrap();
        let anchor = rotated_point(bounds.left_top(), bounds.center(), transform.rotation);
        let drag = DragState {
            start_canvas: rotated_point(bounds.right_bottom(), bounds.center(), transform.rotation),
            current_canvas: anchor + rotate_vector(bounds.size() * 2.0, transform.rotation),
            layer_id: Some(1),
            transform,
            action: DragAction::Resize(ResizeHandle::BottomRight),
            bounds: Some(bounds),
            paragraph_bounds: None,
            paragraph_width: None,
            paragraph_source_override: None,
        };

        let resized = drag_transform(drag, false);
        let resized_bounds = Rect::from_min_size(
            Pos2::new(
                resized.x + source.visual_bounds.left() * resized.scale_x,
                resized.y + source.visual_bounds.top() * resized.scale_y,
            ),
            Vec2::new(
                source.visual_bounds.width() * resized.scale_x,
                source.visual_bounds.height() * resized.scale_y,
            ),
        );
        let resized_anchor = rotated_point(
            resized_bounds.left_top(),
            resized_bounds.center(),
            resized.rotation,
        );

        assert!((resized.scale_x - 2.0).abs() < 0.001);
        assert!((resized.scale_y - 2.0).abs() < 0.001);
        assert!(resized_anchor.distance(anchor) < 0.001);
    }
}
