use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct LayerVisualKey {
    kind: LayerKind,
    adjustments: spectrum_imaging::Adjustments,
}

impl LayerVisualKey {
    pub(super) fn new(layer: &Layer) -> Self {
        Self {
            kind: layer.kind.clone(),
            adjustments: layer.adjustments.clone(),
        }
    }
}

pub(super) enum LayerVisual {
    Solid(Color32),
    Texture(TextureHandle),
}

pub(super) struct LayerVisualEntry {
    key: LayerVisualKey,
    visual: LayerVisual,
    source_size: Vec2,
    max_size: u32,
}

pub(super) struct LayerRenderRequest {
    tab_id: u64,
    layer: Layer,
    key: LayerVisualKey,
    max_size: u32,
}

pub(super) struct LayerRenderResult {
    tab_id: u64,
    layer_id: u64,
    key: LayerVisualKey,
    max_size: u32,
    result: Result<(image::DynamicImage, Vec2), String>,
}

impl PrismApp {
    pub(super) fn reset_canvas_cache(&mut self) {
        self.layer_visuals.clear();
        self.layer_visual_dirty.clear();
        self.layer_render_pending.clear();
        self.preview_error = None;
    }

    pub(super) fn ensure_layer_visuals(&mut self, context: &egui::Context) {
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
                .map(LayerVisualKey::new);
            if current_key.as_ref() != Some(&result.key) {
                continue;
            }
            match result.result {
                Ok((image, source_size)) => {
                    let rgba = image.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    let texture = context.load_texture(
                        format!("prism-layer-preview-{}", result.layer_id),
                        pixels,
                        TextureOptions::LINEAR,
                    );
                    self.layer_visuals.insert(
                        result.layer_id,
                        LayerVisualEntry {
                            key: result.key,
                            visual: LayerVisual::Texture(texture),
                            source_size,
                            max_size: result.max_size,
                        },
                    );
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

        let required_max_size = if self.workspace.interaction_active() {
            1024
        } else {
            2048
        };
        for layer in self
            .workspace
            .document
            .layers
            .iter()
            .filter(|layer| layer.visible)
        {
            let cached_at_quality = self
                .layer_visuals
                .get(&layer.id)
                .is_some_and(|entry| entry.max_size >= required_max_size);
            if cached_at_quality && !self.layer_visual_dirty.contains(&layer.id) {
                self.layer_render_pending.remove(&layer.id);
                continue;
            }
            let key = LayerVisualKey::new(layer);
            let current = self
                .layer_visuals
                .get(&layer.id)
                .is_some_and(|entry| entry.key == key && entry.max_size >= required_max_size);
            if current {
                self.layer_visual_dirty.remove(&layer.id);
                self.layer_render_pending.remove(&layer.id);
                continue;
            }
            if let LayerKind::Rectangle {
                width,
                height,
                color,
                ..
            } = layer.kind
            {
                let adjusted = prism_core::render_solid_color(color, &layer.adjustments);
                self.layer_visuals.insert(
                    layer.id,
                    LayerVisualEntry {
                        key,
                        visual: LayerVisual::Solid(color32(adjusted)),
                        source_size: Vec2::new(width as f32, height as f32),
                        max_size: u32::MAX,
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
                        key,
                        max_size: required_max_size,
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
                if self.layer_render_request_sender.send(request).is_err() {
                    self.layer_render_in_flight = false;
                    self.preview_error = Some("Layer preview worker stopped".into());
                }
            }
        }
    }

    pub(super) fn layer_source_size(&self, layer: &Layer) -> Option<Vec2> {
        self.layer_visuals
            .get(&layer.id)
            .map(|entry| entry.source_size)
            .or_else(|| source_size_before_preview(layer))
    }
}

struct CachedLayerBase {
    kind: LayerKind,
    max_size: u32,
    image: image::DynamicImage,
}

pub(super) fn spawn_layer_render_worker(
    receiver: Receiver<LayerRenderRequest>,
    sender: Sender<LayerRenderResult>,
    repaint: egui::Context,
) {
    std::thread::spawn(move || {
        let mut bases: HashMap<(u64, u64), CachedLayerBase> = HashMap::new();
        while let Ok(request) = receiver.recv() {
            let cache_id = (request.tab_id, request.layer.id);
            let cached = bases.get(&cache_id).filter(|cached| {
                cached.kind == request.layer.kind && cached.max_size >= request.max_size
            });
            let base = if let Some(cached) = cached {
                Ok(cached.image.clone())
            } else {
                prism_core::render_layer_base(&request.layer, Some(request.max_size)).inspect(
                    |image| {
                        bases.insert(
                            cache_id,
                            CachedLayerBase {
                                kind: request.layer.kind.clone(),
                                max_size: request.max_size,
                                image: image.clone(),
                            },
                        );
                    },
                )
            };
            let source_size = source_size_before_preview(&request.layer);
            let result = base
                .map(|image| {
                    spectrum_imaging::render_image(
                        image,
                        request.layer.adjustments.clone(),
                        spectrum_imaging::RenderOptions {
                            max_size: Some(request.max_size),
                        },
                    )
                })
                .map(|image| {
                    let size = source_size
                        .unwrap_or_else(|| Vec2::new(image.width() as f32, image.height() as f32));
                    (image, size)
                })
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(LayerRenderResult {
                tab_id: request.tab_id,
                layer_id: request.layer.id,
                key: request.key,
                max_size: request.max_size,
                result,
            });
            repaint.request_repaint();
        }
    });
}

fn source_size_before_preview(layer: &Layer) -> Option<Vec2> {
    match &layer.kind {
        LayerKind::Raster { path, .. } => image::image_dimensions(path)
            .ok()
            .map(|(width, height)| Vec2::new(width as f32, height as f32)),
        LayerKind::Text {
            text, font_size, ..
        } => {
            let longest = text.lines().map(str::len).max().unwrap_or(1) as f32;
            let lines = text.lines().count().max(1) as f32;
            Some(Vec2::new(
                longest * font_size * 0.55,
                lines * font_size * 1.25,
            ))
        }
        LayerKind::Rectangle { width, height, .. } => {
            Some(Vec2::new(*width as f32, *height as f32))
        }
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
        Stroke::new(1.0, Color32::from_gray(90)),
        egui::StrokeKind::Outside,
    );
}

fn paint_canvas_background(ui: &egui::Ui, geometry: CanvasGeometry, background: [u8; 4]) {
    ui.painter().rect_filled(geometry.viewport, 0.0, INK);
    if background[3] == 255 {
        ui.painter()
            .rect_filled(geometry.canvas, 0.0, color32(background));
        return;
    }
    let clipped = geometry.canvas.intersect(geometry.viewport);
    let checker = 20.0;
    let cols = (clipped.width() / checker).ceil() as i32 + 1;
    let rows = (clipped.height() / checker).ceil() as i32 + 1;
    for row in 0..rows {
        for col in 0..cols {
            let min = clipped.min + Vec2::new(col as f32 * checker, row as f32 * checker);
            let cell = Rect::from_min_size(min, Vec2::splat(checker)).intersect(clipped);
            let color = if (row + col) % 2 == 0 {
                Color32::from_rgb(64, 67, 73)
            } else {
                Color32::from_rgb(49, 52, 58)
            };
            ui.painter().rect_filled(cell, 0.0, color);
        }
    }
    ui.painter().rect_filled(
        geometry.canvas,
        0.0,
        Color32::from_rgba_unmultiplied(background[0], background[1], background[2], background[3]),
    );
}

fn paint_layer_visual(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    layer: &Layer,
    entry: &LayerVisualEntry,
) {
    let canvas_rect = layer_bounds_with_size(layer, entry.source_size);
    let mut screen_rect = Rect::from_min_max(
        geometry.canvas_to_screen(canvas_rect.min),
        geometry.canvas_to_screen(canvas_rect.max),
    );
    let mut uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    if layer.mask.enabled && !layer.mask.invert {
        let size = screen_rect.size();
        screen_rect = Rect::from_min_size(
            screen_rect.min + Vec2::new(size.x * layer.mask.x, size.y * layer.mask.y),
            Vec2::new(size.x * layer.mask.width, size.y * layer.mask.height),
        );
        uv = Rect::from_min_size(
            Pos2::new(layer.mask.x, layer.mask.y),
            Vec2::new(layer.mask.width, layer.mask.height),
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
                uv,
                Color32::from_rgba_unmultiplied(
                    color.r(),
                    color.g(),
                    color.b(),
                    (color.a() as u16 * alpha as u16 / 255) as u8,
                ),
                layer.transform.rotation,
            );
        }
        LayerVisual::Texture(texture) => paint_quad(
            ui,
            geometry.viewport,
            texture.id(),
            screen_rect,
            uv,
            Color32::from_white_alpha(alpha),
            layer.transform.rotation,
        ),
    }
}

fn paint_quad(
    ui: &egui::Ui,
    clip: Rect,
    texture: egui::TextureId,
    rect: Rect,
    uv: Rect,
    color: Color32,
    rotation_degrees: f32,
) {
    let mut positions = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    if rotation_degrees.abs() >= 0.01 {
        let center = rect.center();
        let radians = rotation_degrees.to_radians();
        let (sin, cos) = radians.sin_cos();
        for position in &mut positions {
            let delta = *position - center;
            *position =
                center + Vec2::new(delta.x * cos - delta.y * sin, delta.x * sin + delta.y * cos);
        }
    }
    let uvs = [
        uv.left_top(),
        uv.right_top(),
        uv.right_bottom(),
        uv.left_bottom(),
    ];
    let mut mesh = egui::Mesh::with_texture(texture);
    for (position, uv) in positions.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: position,
            uv,
            color,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    ui.painter().with_clip_rect(clip).add(mesh);
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
