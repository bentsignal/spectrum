use super::*;

pub(super) fn paint_texture_shadow_preview(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    layer: &Layer,
    texture: egui::TextureId,
    rect: Rect,
    uv: Rect,
    rotation_pivot: Option<Pos2>,
) {
    let Some(shadow) = layer.style.drop_shadow else {
        return;
    };
    // The direct-manipulation path deliberately bounds blurred shadows to the core
    // compositor's 13 kernel taps. These tinted quads preserve the requested alpha
    // where every sample overlaps, but they only approximate convolution at partially
    // transparent edges. The exact compositor replaces this preview after the gesture.
    for_each_shadow_preview_sample(shadow, layer.opacity, |canvas_offset, color| {
        let screen_offset = canvas_offset * geometry.pixels_per_point;
        let shifted_rect = Rect::from_min_max(rect.min + screen_offset, rect.max + screen_offset);
        paint_quad(
            ui,
            geometry.viewport,
            texture,
            shifted_rect,
            Some(uv),
            color,
            (
                layer.transform.rotation,
                rotation_pivot.map(|pivot| pivot + screen_offset),
            ),
        );
    });
}

pub(super) fn layer_texture_bounds(layer: &Layer, entry: &LayerVisualEntry) -> Rect {
    if matches!(layer.kind, LayerKind::Path { .. }) {
        let visual = entry.source_geometry.visual_bounds;
        return Rect::from_min_size(
            Pos2::new(
                layer.transform.x + visual.left() * layer.transform.scale_x,
                layer.transform.y + visual.top() * layer.transform.scale_y,
            ),
            Vec2::new(
                visual.width() * layer.transform.scale_x,
                visual.height() * layer.transform.scale_y,
            ),
        );
    }
    if !matches!(layer.kind, LayerKind::Text { .. }) {
        return layer_bounds_with_size(layer, entry.source_geometry.size);
    }
    aligned_text_texture_bounds(layer, entry.source_geometry, entry.texture_visual_bounds)
}

pub(super) fn aligned_text_texture_bounds(
    layer: &Layer,
    source_geometry: LayerSourceGeometry,
    texture_visual: Rect,
) -> Rect {
    let visual = source_geometry.visual_bounds;
    let target = Rect::from_min_size(
        Pos2::new(
            layer.transform.x + visual.left() * layer.transform.scale_x,
            layer.transform.y + visual.top() * layer.transform.scale_y,
        ),
        Vec2::new(
            visual.width() * layer.transform.scale_x,
            visual.height() * layer.transform.scale_y,
        ),
    );
    let texture_size = Vec2::new(
        target.width() / texture_visual.width().max(f32::EPSILON),
        target.height() / texture_visual.height().max(f32::EPSILON),
    );
    Rect::from_min_size(
        target.min
            - Vec2::new(
                texture_visual.left() * texture_size.x,
                texture_visual.top() * texture_size.y,
            ),
        texture_size,
    )
}

pub(super) fn aligned_text_uv(
    source_geometry: LayerSourceGeometry,
    texture_visual: Rect,
    canonical_uv: Rect,
) -> Rect {
    let visual = source_geometry.visual_bounds;
    let map_x = |fraction: f32| {
        texture_visual.left()
            + (fraction * source_geometry.size.x - visual.left()) * texture_visual.width()
                / visual.width().max(f32::EPSILON)
    };
    let map_y = |fraction: f32| {
        texture_visual.top()
            + (fraction * source_geometry.size.y - visual.top()) * texture_visual.height()
                / visual.height().max(f32::EPSILON)
    };
    Rect::from_min_max(
        Pos2::new(
            map_x(canonical_uv.left()).clamp(0.0, 1.0),
            map_y(canonical_uv.top()).clamp(0.0, 1.0),
        ),
        Pos2::new(
            map_x(canonical_uv.right()).clamp(0.0, 1.0),
            map_y(canonical_uv.bottom()).clamp(0.0, 1.0),
        ),
    )
}

pub(super) fn paint_quad(
    ui: &egui::Ui,
    clip: Rect,
    texture: egui::TextureId,
    rect: Rect,
    uv: Option<Rect>,
    color: Color32,
    rotation: (f32, Option<Pos2>),
) {
    let (rotation_degrees, rotation_pivot) = rotation;
    let mesh = quad_mesh(texture, rect, uv, color, rotation_degrees, rotation_pivot);
    ui.painter().with_clip_rect(clip).add(mesh);
}

pub(super) fn quad_mesh(
    texture: egui::TextureId,
    rect: Rect,
    uv: Option<Rect>,
    color: Color32,
    rotation_degrees: f32,
    rotation_pivot: Option<Pos2>,
) -> egui::Mesh {
    let mut positions = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    if rotation_degrees.abs() >= 0.01 {
        let center = rotation_pivot.unwrap_or_else(|| rect.center());
        let (sin, cos) = prism_core::rotation_sin_cos(rotation_degrees);
        for position in &mut positions {
            let delta = *position - center;
            *position =
                center + Vec2::new(delta.x * cos - delta.y * sin, delta.x * sin + delta.y * cos);
        }
    }
    let mut mesh = egui::Mesh::with_texture(texture);
    if let Some(uv) = uv {
        let uvs = [
            uv.left_top(),
            uv.right_top(),
            uv.right_bottom(),
            uv.left_bottom(),
        ];
        for (position, uv) in positions.into_iter().zip(uvs) {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: position,
                uv,
                color,
            });
        }
    } else {
        for position in positions {
            mesh.colored_vertex(position, color);
        }
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    mesh
}

pub(super) fn layer_bounds_with_size(layer: &Layer, source_size: Vec2) -> Rect {
    Rect::from_min_size(
        Pos2::new(layer.transform.x, layer.transform.y),
        Vec2::new(
            source_size.x * layer.transform.scale_x,
            source_size.y * layer.transform.scale_y,
        ),
    )
}
