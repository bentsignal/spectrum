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
    let stroke = Stroke::new(1.5, if enabled { ACCENT } else { MUTED });
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

pub(super) fn paint_layer_outline(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    layer: &Layer,
    source_size: Option<Vec2>,
    offset: Vec2,
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

pub(super) fn contrast_text(background: Color32) -> Color32 {
    let luma = background.r() as u16 * 3 + background.g() as u16 * 6 + background.b() as u16;
    if luma > 1_400 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

pub(super) fn color32(value: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(value[0], value[1], value[2], value[3])
}

pub(super) fn rgba(value: Color32) -> [u8; 4] {
    [value.r(), value.g(), value.b(), value.a()]
}
