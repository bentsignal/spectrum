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
                            == cached_base_pixel_mask_identity(&render_layer)
                        && cached.shape_raster_scale == request.key.shape_raster_scale
                        && cached_base_max_size_is_compatible(
                            &render_layer,
                            cached.max_size,
                            request.max_size,
                        )
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
                            pixel_mask_identity: cached_base_pixel_mask_identity(&render_layer),
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
                    prism_core::render_layer_preview_from_base(
                        &render_layer,
                        image,
                        Some(request.max_size),
                    )
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

fn cached_base_pixel_mask_identity(layer: &Layer) -> Option<[u8; 32]> {
    if matches!(layer.kind, LayerKind::Raster { .. }) {
        None
    } else {
        layer
            .pixel_mask
            .as_ref()
            .map(prism_core::PixelMask::identity)
    }
}

fn cached_base_max_size_is_compatible(layer: &Layer, cached: u32, requested: u32) -> bool {
    if matches!(layer.kind, LayerKind::Raster { .. }) && layer.pixel_mask.is_some() {
        cached == requested
    } else {
        cached >= requested
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_bases_ignore_masks_but_shape_bases_keep_mask_identity() {
        let mask = prism_core::PixelMask::new(2, 1, vec![255, 0]);
        let raster = Layer {
            pixel_mask: Some(mask.clone()),
            kind: LayerKind::Raster {
                path: "source.png".into(),
                original_path: None,
            },
            ..Layer::default()
        };
        assert_eq!(cached_base_pixel_mask_identity(&raster), None);

        let shape = Layer {
            pixel_mask: Some(mask.clone()),
            kind: LayerKind::Rectangle {
                width: 2,
                height: 1,
                color: [255; 4],
                corner_radius: 0.0,
            },
            ..Layer::default()
        };
        assert_eq!(
            cached_base_pixel_mask_identity(&shape),
            Some(mask.identity())
        );
    }

    #[test]
    fn masked_raster_bases_do_not_reuse_a_differently_resized_cache_entry() {
        let masked_raster = Layer {
            pixel_mask: Some(prism_core::PixelMask::new(16, 12, vec![255; 192])),
            kind: LayerKind::Raster {
                path: "source.png".into(),
                original_path: None,
            },
            ..Layer::default()
        };
        assert!(!cached_base_max_size_is_compatible(&masked_raster, 8, 4));
        assert!(cached_base_max_size_is_compatible(&masked_raster, 4, 4));

        let mut unmasked_raster = masked_raster;
        unmasked_raster.pixel_mask = None;
        assert!(cached_base_max_size_is_compatible(&unmasked_raster, 8, 4));
    }
}
