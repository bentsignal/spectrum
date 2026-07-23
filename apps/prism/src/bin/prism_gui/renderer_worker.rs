use super::*;

struct CachedLayerBase {
    kind: LayerKind,
    stroke: ShapeStroke,
    shape_fill: Option<prism_core::ShapeFill>,
    pixel_mask_identity: Option<[u8; 32]>,
    max_size: u32,
    shape_raster_scale: [u32; 2],
    image: image::DynamicImage,
}

pub(crate) fn spawn_layer_render_worker(
    receiver: Receiver<LayerRenderMessage>,
    sender: Sender<LayerRenderResult>,
    repaint: egui::Context,
) {
    std::thread::spawn(move || {
        let mut bases: BoundedLruCache<(u64, u64), CachedLayerBase> =
            BoundedLruCache::new(MAX_WORKER_BASE_BYTES);
        while let Ok(message) = receiver.recv() {
            let request = match message {
                LayerRenderMessage::Render(request) => *request,
                LayerRenderMessage::Prune(active) => {
                    bases.retain(&active);
                    continue;
                }
            };
            let cache_id = (request.tab_id, request.layer.id);
            let mut render_layer = request.layer.clone();
            if let LayerKind::Text {
                font_size,
                typography,
                ..
            } = &mut render_layer.kind
            {
                *font_size *= request.key.text_raster_scale as f32;
                typography.scale_for_raster(request.key.text_raster_scale as f32);
            }
            let texture_visual_bounds = if matches!(render_layer.kind, LayerKind::Path { .. }) {
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
            } else {
                source_geometry_before_preview(&render_layer, request.font_asset.as_ref())
                    .map(normalized_visual_bounds)
                    .unwrap_or_else(|| Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)))
            };
            let cached = bases
                .get(cache_id)
                .filter(|cached| {
                    cached.kind == render_layer.kind
                        && cached.stroke == render_layer.stroke
                        && cached.shape_fill == render_layer.shape_fill
                        && cached.pixel_mask_identity
                            == render_layer
                                .pixel_mask
                                .as_ref()
                                .map(prism_core::PixelMask::identity)
                        && cached.shape_raster_scale == request.key.shape_raster_scale
                        && cached.max_size >= request.max_size
                })
                .map(|cached| cached.image.clone());
            let paint_final = matches!(render_layer.kind, LayerKind::Paint { .. });
            let base = if paint_final {
                prism_core::render_layer_preview_scaled_with_font(
                    &render_layer,
                    Some(request.max_size),
                    [1.0; 2],
                    request.font_asset.as_ref(),
                )
            } else if let Some(cached) = cached {
                Ok(cached)
            } else {
                prism_core::render_layer_base_scaled_with_font(
                    &render_layer,
                    Some(request.max_size),
                    [
                        request.key.shape_raster_scale[0] as f32,
                        request.key.shape_raster_scale[1] as f32,
                    ],
                    request.font_asset.as_ref(),
                )
                .inspect(|image| {
                    bases.insert(
                        cache_id,
                        CachedLayerBase {
                            kind: render_layer.kind.clone(),
                            stroke: render_layer.stroke,
                            shape_fill: render_layer.shape_fill.clone(),
                            pixel_mask_identity: render_layer
                                .pixel_mask
                                .as_ref()
                                .map(prism_core::PixelMask::identity),
                            max_size: request.max_size,
                            shape_raster_scale: request.key.shape_raster_scale,
                            image: image.clone(),
                        },
                        rgba_bytes(image.width(), image.height()),
                    );
                })
            };
            let source_geometry =
                source_geometry_before_preview(&request.layer, request.font_asset.as_ref());
            let result = base
                .and_then(|image| {
                    if paint_final {
                        return Ok(image);
                    }
                    let mut image = spectrum_imaging::render_image(
                        image,
                        request.layer.adjustments.clone(),
                        spectrum_imaging::RenderOptions {
                            max_size: Some(request.max_size),
                        },
                    )
                    .to_rgba8();
                    let (width, height) = image.dimensions();
                    prism_core::apply_vector_mask_to_image(
                        &mut image,
                        request.layer.vector_mask.as_ref(),
                        width,
                        height,
                        0,
                        0,
                    )?;
                    Ok(image::DynamicImage::ImageRgba8(image))
                })
                .map(|image| {
                    let geometry = source_geometry.unwrap_or_else(|| {
                        LayerSourceGeometry::full(Vec2::new(
                            image.width() as f32,
                            image.height() as f32,
                        ))
                    });
                    (image, geometry, texture_visual_bounds)
                })
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(LayerRenderResult {
                tab_id: request.tab_id,
                layer_id: request.layer.id,
                key: request.key,
                max_size: request.max_size,
                resident_byte_budget: request.resident_byte_budget,
                result,
            });
            repaint.request_repaint();
        }
    });
}
