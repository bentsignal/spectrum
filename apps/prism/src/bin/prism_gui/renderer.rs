use super::*;

#[path = "renderer_worker.rs"]
mod worker;
pub(super) use worker::spawn_layer_render_worker;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct LayerVisualKey {
    pub(super) kind: LayerKind,
    adjustments: spectrum_imaging::Adjustments,
    stroke: ShapeStroke,
    shape_fill: Option<prism_core::ShapeFill>,
    pixel_mask_identity: Option<[u8; 32]>,
    vector_mask_identity: Option<[u8; 32]>,
    textured_shape_preview: bool,
    colored_shadow_mask: bool,
    pub(super) text_raster_scale: u32,
    pub(super) shape_raster_scale: [u32; 2],
}

impl LayerVisualKey {
    pub(super) fn new(layer: &Layer, display_scale: f32) -> Self {
        Self {
            kind: layer.kind.clone(),
            adjustments: layer.adjustments.clone(),
            stroke: layer.stroke,
            shape_fill: layer.shape_fill.clone(),
            pixel_mask_identity: layer
                .pixel_mask
                .as_ref()
                .map(prism_core::PixelMask::identity),
            vector_mask_identity: layer
                .vector_mask
                .as_ref()
                .map(prism_core::VectorMask::identity),
            textured_shape_preview: textured_shape_preview(layer),
            colored_shadow_mask: colored_shadow_mask(layer),
            text_raster_scale: preview_text_scale(layer, display_scale),
            shape_raster_scale: prism_core::interactive_shape_scale(layer, display_scale)
                .unwrap_or([1; 2]),
        }
    }
}

pub(super) fn desired_layer_visual_key(
    layer: &Layer,
    display_scale: f32,
    interaction_active: bool,
    cached: Option<&LayerVisualKey>,
) -> LayerVisualKey {
    let mut key = LayerVisualKey::new(layer, display_scale);
    if matches!(layer.kind, LayerKind::Text { .. })
        && let Some(cached) = cached
    {
        if interaction_active {
            key.text_raster_scale = cached.text_raster_scale;
        } else if cached.text_raster_scale == key.text_raster_scale.saturating_mul(2)
            && text_display_scale_target(layer, display_scale)
                > key.text_raster_scale as f32 * 0.875
        {
            // Keep the higher tier through a narrow dead band while zooming out.
            // Zooming in still upgrades immediately, so settled text is never
            // magnified beyond its requested source resolution.
            key.text_raster_scale = cached.text_raster_scale;
        }
    }
    key
}

pub(super) enum LayerVisual {
    Solid(Color32),
    Texture(TextureHandle),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LayerSourceGeometry {
    pub(super) size: Vec2,
    pub(super) visual_bounds: Rect,
    pub(super) paragraph_bounds: Option<Rect>,
}

impl LayerSourceGeometry {
    pub(super) fn full(size: Vec2) -> Self {
        Self {
            size,
            visual_bounds: Rect::from_min_size(Pos2::ZERO, size),
            paragraph_bounds: None,
        }
    }
}

pub(super) fn text_source_geometry(
    geometry: prism_core::TextGeometry,
    paragraph: bool,
) -> LayerSourceGeometry {
    LayerSourceGeometry {
        size: Vec2::new(geometry.width as f32, geometry.height as f32),
        visual_bounds: Rect::from_min_size(
            Pos2::new(geometry.visual_left, geometry.visual_top),
            Vec2::new(geometry.visual_width, geometry.visual_height),
        ),
        paragraph_bounds: paragraph.then(|| {
            Rect::from_min_size(
                Pos2::new(geometry.layout_left, geometry.layout_top),
                Vec2::new(geometry.layout_width, geometry.layout_height),
            )
        }),
    }
}

pub(super) struct LayerVisualEntry {
    key: LayerVisualKey,
    visual: LayerVisual,
    shadow_mask: Option<TextureHandle>,
    source_geometry: LayerSourceGeometry,
    texture_visual_bounds: Rect,
    max_size: u32,
    texture_bytes: u64,
}

pub(super) struct LayerRenderRequest {
    tab_id: u64,
    layer: Layer,
    font_asset: Option<prism_core::FontAsset>,
    key: LayerVisualKey,
    max_size: u32,
    resident_byte_budget: u64,
}

pub(super) struct LayerRenderResult {
    tab_id: u64,
    layer_id: u64,
    key: LayerVisualKey,
    max_size: u32,
    resident_byte_budget: u64,
    result: Result<(image::DynamicImage, LayerSourceGeometry, Rect), String>,
}

pub(super) enum LayerRenderMessage {
    Render(Box<LayerRenderRequest>),
    Prune(HashSet<(u64, u64)>),
}

pub(super) fn reuse_cached_visual_during_interaction(
    cached_max_size: Option<u32>,
    required_max_size: u32,
    dirty: bool,
    interaction_active: bool,
) -> bool {
    interaction_active
        && !dirty
        && cached_max_size.is_some_and(|max_size| max_size >= required_max_size)
}

fn make_texture_budget_room(
    visuals: &mut HashMap<u64, LayerVisualEntry>,
    replacement_id: u64,
    replacement_bytes: u64,
) {
    visuals.remove(&replacement_id);
    while visuals
        .values()
        .map(|entry| entry.texture_bytes)
        .sum::<u64>()
        .saturating_add(replacement_bytes)
        > MAX_PREVIEW_TEXTURE_RESIDENT_BYTES
    {
        let Some(largest) = visuals
            .iter()
            .max_by_key(|(_, entry)| entry.texture_bytes)
            .map(|(id, _)| *id)
        else {
            break;
        };
        visuals.remove(&largest);
    }
}

impl PrismApp {
    pub(super) fn reset_canvas_cache(&mut self) {
        self.layer_visuals.clear();
        self.layer_source_overrides.clear();
        self.text_source_geometries.clear();
        self.layer_visual_dirty.clear();
        self.layer_render_pending.clear();
        self.preview_error = None;
        self.composite_preview.reset();
        self.sync_layer_render_cache_scope();
    }

    pub(super) fn sync_layer_render_cache_scope(&mut self) {
        let active = self
            .workspace
            .document
            .layers
            .iter()
            .map(|layer| (self.active_tab_id, layer.id))
            .collect::<HashSet<_>>();
        if active == self.layer_render_active_cache_ids {
            return;
        }
        self.layer_render_active_cache_ids = active.clone();
        let _ = self
            .layer_render_request_sender
            .send(LayerRenderMessage::Prune(active));
    }

    pub(super) fn ensure_layer_visuals(&mut self, context: &egui::Context, display_scale: f32) {
        let runtime_max_texture_side = context.input(|input| input.max_texture_side);
        while let Ok(result) = self.layer_render_receiver.try_recv() {
            self.layer_render_in_flight = false;
            if result.tab_id != self.active_tab_id {
                continue;
            }
            let current_key = self
                .workspace
                .document
                .layers
                .iter()
                .find(|layer| layer.id == result.layer_id)
                .map(|layer| {
                    desired_layer_visual_key(
                        layer,
                        display_scale,
                        self.workspace.interaction_active(),
                        self.layer_visuals
                            .get(&result.layer_id)
                            .map(|entry| &entry.key),
                    )
                });
            if current_key.as_ref() != Some(&result.key) {
                continue;
            }
            match result.result {
                Ok((image, source_geometry, texture_visual_bounds)) => {
                    let rgba = image.to_rgba8();
                    let texture_bytes = preview_texture_resident_bytes(
                        rgba.width(),
                        rgba.height(),
                        result.key.colored_shadow_mask,
                    );
                    if !preview_upload_allowed(
                        rgba.width(),
                        rgba.height(),
                        runtime_max_texture_side,
                        result.resident_byte_budget,
                        result.key.colored_shadow_mask,
                    ) {
                        self.preview_error =
                            Some("Layer preview exceeds the bounded texture budget".into());
                        continue;
                    }
                    make_texture_budget_room(
                        &mut self.layer_visuals,
                        result.layer_id,
                        texture_bytes,
                    );
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    let texture = context.load_texture(
                        format!("prism-layer-preview-{}", result.layer_id),
                        pixels,
                        TextureOptions::LINEAR,
                    );
                    let shadow_mask = result.key.colored_shadow_mask.then(|| {
                        let mask = bounded_shadow_mask(&rgba);
                        let mask_size = [mask.width() as usize, mask.height() as usize];
                        context.load_texture(
                            format!("prism-layer-shadow-mask-{}", result.layer_id),
                            egui::ColorImage::from_rgba_unmultiplied(mask_size, mask.as_raw()),
                            TextureOptions::LINEAR,
                        )
                    });
                    let is_text = matches!(&result.key.kind, LayerKind::Text { .. });
                    self.layer_visuals.insert(
                        result.layer_id,
                        LayerVisualEntry {
                            key: result.key,
                            visual: LayerVisual::Texture(texture),
                            shadow_mask,
                            source_geometry,
                            texture_visual_bounds,
                            max_size: result.max_size,
                            texture_bytes,
                        },
                    );
                    if is_text {
                        self.text_source_geometries
                            .insert(result.layer_id, source_geometry);
                    }
                    self.layer_source_overrides.remove(&result.layer_id);
                    self.layer_visual_dirty.remove(&result.layer_id);
                    self.preview_error = None;
                }
                Err(error) => self.preview_error = Some(error),
            }
        }

        let active_ids: HashSet<_> = self
            .workspace
            .document
            .layers
            .iter()
            .map(|layer| layer.id)
            .collect();
        self.layer_visuals.retain(|id, _| active_ids.contains(id));
        self.text_source_geometries
            .retain(|id, _| active_ids.contains(id));
        self.sync_layer_render_cache_scope();
        let visible_ids: HashSet<_> = self
            .workspace
            .document
            .layers
            .iter()
            .filter(|layer| layer.visible)
            .map(|layer| layer.id)
            .collect();
        self.layer_render_pending
            .retain(|id, _| visible_ids.contains(id));

        let interaction_active = self.workspace.interaction_active();
        let visible_texture_layers = self
            .workspace
            .document
            .layers
            .iter()
            .filter(|layer| layer.visible && solid_preview(layer).is_none())
            .count();
        let layer_byte_budget = per_layer_texture_bytes(visible_texture_layers);
        for layer in self
            .workspace
            .document
            .layers
            .iter()
            .filter(|layer| layer.visible)
        {
            let source_geometry = self.text_source_geometries.get(layer.id).copied();
            let scheduling = self.text_source_geometries.schedule_layer(
                layer,
                interaction_active,
                self.layer_visuals.contains_key(&layer.id),
                self.layer_visual_dirty.contains(&layer.id),
                || {
                    let key = desired_layer_visual_key(
                        layer,
                        display_scale,
                        interaction_active,
                        self.layer_visuals.get(&layer.id).map(|entry| &entry.key),
                    );
                    let font_asset = self.workspace.document.font_for_layer(layer);
                    let required_max_size = required_preview_size(
                        layer,
                        &key,
                        interaction_active,
                        source_geometry,
                        runtime_max_texture_side,
                        layer_byte_budget,
                    );
                    (key, font_asset, required_max_size)
                },
            );
            let (key, font_asset, required_max_size) = match scheduling {
                LayerPreviewSchedule::ReuseCachedTextVisual => {
                    self.layer_render_pending.remove(&layer.id);
                    continue;
                }
                LayerPreviewSchedule::Resolve(resolved) => resolved,
            };
            if reuse_cached_visual_during_interaction(
                self.layer_visuals
                    .get(&layer.id)
                    .map(|entry| entry.max_size),
                required_max_size,
                self.layer_visual_dirty.contains(&layer.id),
                interaction_active,
            ) {
                self.layer_render_pending.remove(&layer.id);
                continue;
            }
            let current = self
                .layer_visuals
                .get(&layer.id)
                .is_some_and(|entry| entry.key == key && entry.max_size >= required_max_size);
            if current {
                self.layer_visual_dirty.remove(&layer.id);
                self.layer_render_pending.remove(&layer.id);
                continue;
            }
            if let Some((width, height, color)) = solid_preview(layer) {
                let adjusted = prism_core::render_solid_color(color, &layer.adjustments);
                self.layer_visuals.insert(
                    layer.id,
                    LayerVisualEntry {
                        key,
                        visual: LayerVisual::Solid(color32(adjusted)),
                        shadow_mask: None,
                        source_geometry: LayerSourceGeometry::full(Vec2::new(
                            width as f32,
                            height as f32,
                        )),
                        texture_visual_bounds: Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        max_size: u32::MAX,
                        texture_bytes: 0,
                    },
                );
                self.layer_visual_dirty.remove(&layer.id);
                self.layer_render_pending.remove(&layer.id);
            } else {
                self.layer_render_pending.insert(
                    layer.id,
                    LayerRenderRequest {
                        tab_id: self.active_tab_id,
                        layer: layer.clone(),
                        font_asset: font_asset.cloned(),
                        key,
                        max_size: required_max_size,
                        resident_byte_budget: layer_byte_budget,
                    },
                );
            }
        }

        if !self.layer_render_in_flight {
            let next_id = self.workspace.document.layers.iter().find_map(|layer| {
                self.layer_render_pending
                    .contains_key(&layer.id)
                    .then_some(layer.id)
            });
            if let Some(id) = next_id
                && let Some(request) = self.layer_render_pending.remove(&id)
            {
                self.layer_render_in_flight = true;
                if self
                    .layer_render_request_sender
                    .send(LayerRenderMessage::Render(Box::new(request)))
                    .is_err()
                {
                    self.layer_render_in_flight = false;
                    self.preview_error = Some("Layer preview worker stopped".into());
                }
            }
        }
    }

    pub(super) fn layer_source_size(&self, layer: &Layer) -> Option<Vec2> {
        self.layer_source_geometry(layer)
            .map(|geometry| geometry.size)
    }

    pub(super) fn layer_source_geometry(&self, layer: &Layer) -> Option<LayerSourceGeometry> {
        let source_override = self.layer_source_overrides.get(&layer.id);
        let cached = self
            .layer_visuals
            .get(&layer.id)
            .map(|entry| (&entry.key, entry.source_geometry));
        let cached_text = self.text_source_geometries.get(layer.id).copied();
        if matches!(layer.kind, LayerKind::Text { .. })
            && source_override.is_none()
            && cached.is_none()
        {
            return cached_text;
        }
        current_layer_source_geometry(
            layer,
            source_override,
            cached,
            self.workspace.document.font_for_layer(layer),
        )
    }
}

fn normalized_visual_bounds(geometry: LayerSourceGeometry) -> Rect {
    Rect::from_min_max(
        Pos2::new(
            geometry.visual_bounds.left() / geometry.size.x.max(1.0),
            geometry.visual_bounds.top() / geometry.size.y.max(1.0),
        ),
        Pos2::new(
            geometry.visual_bounds.right() / geometry.size.x.max(1.0),
            geometry.visual_bounds.bottom() / geometry.size.y.max(1.0),
        ),
    )
}

pub(super) fn source_geometry_before_preview(
    layer: &Layer,
    font_asset: Option<&prism_core::FontAsset>,
) -> Option<LayerSourceGeometry> {
    match &layer.kind {
        LayerKind::Raster { path, .. } => {
            image::image_dimensions(path).ok().map(|(width, height)| {
                LayerSourceGeometry::full(Vec2::new(width as f32, height as f32))
            })
        }
        LayerKind::Text {
            text,
            font_size,
            typography,
            ..
        } => prism_core::measure_text_geometry_with_typography(
            text, *font_size, typography, font_asset,
        )
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
    }
}

fn preview_text_scale(layer: &Layer, zoom: f32) -> u32 {
    prism_core::recommended_text_raster_scale(layer, zoom) as u32
}

fn text_display_scale_target(layer: &Layer, display_scale: f32) -> f32 {
    layer
        .transform
        .scale_x
        .abs()
        .max(layer.transform.scale_y.abs())
        * display_scale.max(0.1)
}

fn required_preview_size(
    layer: &Layer,
    key: &LayerVisualKey,
    interaction_active: bool,
    source_geometry: Option<LayerSourceGeometry>,
    runtime_max_texture_side: usize,
    layer_byte_budget: u64,
) -> u32 {
    if let Some((width, height)) = prism_core::shape_dimensions(layer) {
        return bounded_preview_max_size(
            width.saturating_mul(key.shape_raster_scale[0]),
            height.saturating_mul(key.shape_raster_scale[1]),
            1,
            runtime_max_texture_side,
            resident_texture_copies(key.colored_shadow_mask),
            layer_byte_budget,
        );
    }
    if matches!(layer.kind, LayerKind::Text { .. }) {
        let (width, height) = source_geometry.map_or((2_048, 2_048), |geometry| {
            (
                geometry.size.x.ceil().max(1.0) as u32,
                geometry.size.y.ceil().max(1.0) as u32,
            )
        });
        let settled = bounded_preview_max_size(
            width,
            height,
            key.text_raster_scale,
            runtime_max_texture_side,
            resident_texture_copies(key.colored_shadow_mask),
            layer_byte_budget,
        );
        return if interaction_active {
            settled.min(1_024)
        } else {
            settled
        };
    }
    if interaction_active { 1024 } else { 2048 }
}

fn textured_shape_preview(layer: &Layer) -> bool {
    matches!(
        layer.kind,
        LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } | LayerKind::Path { .. }
    ) && (layer.shape_fill.is_some() || layer.style.drop_shadow.is_some())
}

fn colored_shadow_mask(layer: &Layer) -> bool {
    layer.style.drop_shadow.is_some_and(|shadow| {
        shadow.color[3] > 0 && shadow.color[..3].iter().any(|channel| *channel > 0)
    })
}

fn solid_preview(layer: &Layer) -> Option<(u32, u32, [u8; 4])> {
    if layer.stroke.enabled
        || layer.pixel_mask.is_some()
        || layer.vector_mask.is_some()
        || textured_shape_preview(layer)
    {
        return None;
    }
    match &layer.kind {
        LayerKind::Rectangle {
            width,
            height,
            color,
            ..
        } => Some((*width, *height, *color)),
        _ => None,
    }
}

pub(super) fn paint_interactive_document(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    document: &Document,
    visuals: &HashMap<u64, LayerVisualEntry>,
) {
    paint_canvas_background(ui, geometry, document.background);
    for layer in document
        .layers
        .iter()
        .filter(|layer| layer.visible && layer.opacity > 0.0)
    {
        let Some(entry) = visuals.get(&layer.id) else {
            continue;
        };
        paint_layer_visual(ui, geometry, layer, entry);
    }
    ui.painter().rect_stroke(
        geometry.canvas,
        1.0,
        Stroke::new(1.0, CANVAS_EDGE),
        egui::StrokeKind::Outside,
    );
}

pub(super) fn paint_canvas_background(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    background: [u8; 4],
) {
    if background[3] == 255 {
        ui.painter().rect_filled(geometry.viewport, 0.0, INK);
        ui.painter()
            .rect_filled(geometry.canvas, 0.0, color32(background));
        return;
    }
    paint_transparency_background(ui, geometry);
    ui.painter().rect_filled(
        geometry.canvas,
        0.0,
        Color32::from_rgba_unmultiplied(background[0], background[1], background[2], background[3]),
    );
}

pub(super) fn paint_transparency_background(ui: &egui::Ui, geometry: CanvasGeometry) {
    ui.painter().rect_filled(geometry.viewport, 0.0, INK);
    let clipped = geometry.canvas.intersect(geometry.viewport);
    let checker = 20.0;
    let cols = (clipped.width() / checker).ceil() as i32 + 1;
    let rows = (clipped.height() / checker).ceil() as i32 + 1;
    for row in 0..rows {
        for col in 0..cols {
            let min = clipped.min + Vec2::new(col as f32 * checker, row as f32 * checker);
            let cell = Rect::from_min_size(min, Vec2::splat(checker)).intersect(clipped);
            let color = if (row + col) % 2 == 0 {
                CHECKER_LIGHT
            } else {
                CHECKER_DARK
            };
            ui.painter().rect_filled(cell, 0.0, color);
        }
    }
}

fn paint_layer_visual(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    layer: &Layer,
    entry: &LayerVisualEntry,
) {
    let canvas_rect = layer_texture_bounds(layer, entry);
    let text_rotation_pivot = if matches!(layer.kind, LayerKind::Text { .. }) {
        layer_bounds(layer, Some(entry.source_geometry))
            .map(|bounds| geometry.canvas_to_screen(bounds.center()))
    } else {
        None
    };
    let mut screen_rect = Rect::from_min_max(
        geometry.canvas_to_screen(canvas_rect.min),
        geometry.canvas_to_screen(canvas_rect.max),
    );
    let mut uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    if layer.mask.enabled && !layer.mask.invert {
        let canonical_uv = Rect::from_min_size(
            Pos2::new(layer.mask.x, layer.mask.y),
            Vec2::new(layer.mask.width, layer.mask.height),
        );
        uv = if matches!(layer.kind, LayerKind::Text { .. }) {
            aligned_text_uv(
                entry.source_geometry,
                entry.texture_visual_bounds,
                canonical_uv,
            )
        } else {
            canonical_uv
        };
        let size = screen_rect.size();
        screen_rect = Rect::from_min_size(
            screen_rect.min + Vec2::new(size.x * uv.left(), size.y * uv.top()),
            Vec2::new(size.x * uv.width(), size.y * uv.height()),
        );
    }
    let alpha = (layer.opacity * 255.0).round().clamp(0.0, 255.0) as u8;
    match &entry.visual {
        LayerVisual::Solid(color) if layer.transform.rotation.abs() < 0.01 => {
            let color = Color32::from_rgba_unmultiplied(
                color.r(),
                color.g(),
                color.b(),
                (color.a() as u16 * alpha as u16 / 255) as u8,
            );
            let radius = match layer.kind {
                LayerKind::Rectangle { corner_radius, .. } => {
                    corner_radius
                        * geometry.pixels_per_point
                        * layer.transform.scale_x.min(layer.transform.scale_y)
                }
                _ => 0.0,
            };
            ui.painter()
                .with_clip_rect(geometry.viewport)
                .rect_filled(screen_rect, radius, color);
        }
        LayerVisual::Solid(color) => {
            paint_quad(
                ui,
                geometry.viewport,
                egui::TextureId::default(),
                screen_rect,
                None,
                Color32::from_rgba_unmultiplied(
                    color.r(),
                    color.g(),
                    color.b(),
                    (color.a() as u16 * alpha as u16 / 255) as u8,
                ),
                (layer.transform.rotation, None),
            );
        }
        LayerVisual::Texture(texture) => {
            paint_texture_shadow_preview(
                ui,
                geometry,
                layer,
                entry
                    .shadow_mask
                    .as_ref()
                    .map_or(texture.id(), TextureHandle::id),
                screen_rect,
                uv,
                text_rotation_pivot,
            );
            paint_quad(
                ui,
                geometry.viewport,
                texture.id(),
                screen_rect,
                Some(uv),
                Color32::from_white_alpha(alpha),
                (layer.transform.rotation, text_rotation_pivot),
            );
        }
    }
}

fn paint_texture_shadow_preview(
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

fn layer_texture_bounds(layer: &Layer, entry: &LayerVisualEntry) -> Rect {
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

fn aligned_text_texture_bounds(
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

fn aligned_text_uv(
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

fn paint_quad(
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

fn quad_mesh(
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

#[cfg(test)]
#[path = "renderer_tests.rs"]
mod tests;
