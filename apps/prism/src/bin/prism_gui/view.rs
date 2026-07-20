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

pub(super) fn layer_bounds(layer: &Layer, cached_size: Option<Vec2>) -> Option<Rect> {
    let size = cached_size.unwrap_or(match &layer.kind {
        LayerKind::Raster { path, .. } => {
            let (width, height) = image::image_dimensions(path).ok()?;
            Vec2::new(width as f32, height as f32)
        }
        LayerKind::Text {
            text, font_size, ..
        } => prism_core::measure_text(text, *font_size)
            .ok()
            .map(|(width, height)| Vec2::new(width as f32, height as f32))?,
        LayerKind::Rectangle { width, height, .. } => Vec2::new(*width as f32, *height as f32),
        LayerKind::Ellipse { width, height, .. } => Vec2::new(*width as f32, *height as f32),
    });
    let size = Vec2::new(
        size.x * layer.transform.scale_x,
        size.y * layer.transform.scale_y,
    );
    Some(Rect::from_min_size(
        Pos2::new(layer.transform.x, layer.transform.y),
        size,
    ))
}

pub(super) fn resize_handle_at(
    geometry: CanvasGeometry,
    layer: &Layer,
    source_size: Option<Vec2>,
    pointer: Pos2,
) -> Option<ResizeHandle> {
    let bounds = layer_bounds(layer, source_size)?;
    let corners = [
        (ResizeHandle::TopLeft, bounds.left_top()),
        (ResizeHandle::TopRight, bounds.right_top()),
        (ResizeHandle::BottomLeft, bounds.left_bottom()),
        (ResizeHandle::BottomRight, bounds.right_bottom()),
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
    let minimum_width = 1.0_f32.min(bounds.width());
    let minimum_height = 1.0_f32.min(bounds.height());
    let width = match handle {
        ResizeHandle::TopLeft | ResizeHandle::BottomLeft => bounds.right() - drag.current_canvas.x,
        ResizeHandle::TopRight | ResizeHandle::BottomRight => drag.current_canvas.x - bounds.left(),
    }
    .max(minimum_width);
    let height = match handle {
        ResizeHandle::TopLeft | ResizeHandle::TopRight => bounds.bottom() - drag.current_canvas.y,
        ResizeHandle::BottomLeft | ResizeHandle::BottomRight => {
            drag.current_canvas.y - bounds.top()
        }
    }
    .max(minimum_height);
    let mut ratio_x = width / bounds.width().max(0.001);
    let mut ratio_y = height / bounds.height().max(0.001);
    if preserve_aspect {
        let ratio = ((ratio_x + ratio_y) * 0.5).max(0.01);
        ratio_x = ratio;
        ratio_y = ratio;
    }
    let width = bounds.width() * ratio_x;
    let height = bounds.height() * ratio_y;
    let (x, y) = match handle {
        ResizeHandle::TopLeft => (bounds.right() - width, bounds.bottom() - height),
        ResizeHandle::TopRight => (bounds.left(), bounds.bottom() - height),
        ResizeHandle::BottomLeft => (bounds.right() - width, bounds.top()),
        ResizeHandle::BottomRight => (bounds.left(), bounds.top()),
    };
    Transform {
        x,
        y,
        scale_x: (drag.transform.scale_x * ratio_x).max(0.01),
        scale_y: (drag.transform.scale_y * ratio_y).max(0.01),
        rotation: drag.transform.rotation,
    }
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
    source_size: Option<Vec2>,
    offset: Vec2,
) {
    paint_selection_outline(ui, geometry, layer, source_size, offset, true);
}

pub(super) fn paint_rotation_outline(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    layer: &Layer,
    source_size: Option<Vec2>,
) {
    paint_selection_outline(ui, geometry, layer, source_size, Vec2::ZERO, false);
}

fn paint_selection_outline(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    layer: &Layer,
    source_size: Option<Vec2>,
    offset: Vec2,
    show_resize_handles: bool,
) {
    let Some(bounds) = layer_bounds(layer, source_size) else {
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
            ui.painter().rect_filled(
                Rect::from_center_size(corner, Vec2::splat(RESIZE_HANDLE_SIZE)),
                1.0,
                ACCENT,
            );
        }
    }
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
}
