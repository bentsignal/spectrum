//! Drawing a layer from its staged source: its drop shadow and styles,
//! then its pixels, within a render region, rows in parallel.
use anyhow::Result;
use image::RgbaImage;
use rayon::prelude::*;

use super::{
    CanvasIntersection, SamplingGeometry, ShadowAlphaTile, effects_tile, sample_output_alpha,
    source::{SampleSource, sample_triangle_resize},
};
use crate::{
    Layer, RegionRenderStats, RenderRegion,
    effects::{colored_shadow_pixel, drop_shadow_alpha},
    effects_render::composite_effect_passes,
    render::{blend_pixel, layer_pixel_alpha},
};

/// Draws a layer from its staged (and resized) source: its shadow and
/// styles, then its pixels, within `region`.
#[allow(clippy::too_many_arguments)]
pub(super) fn composite_staged(
    canvas: &mut RgbaImage,
    coverage: &mut RgbaImage,
    base_layer: &Layer,
    scaled_layer: &Layer,
    clip: Option<&RgbaImage>,
    region: RenderRegion,
    source: SampleSource<'_>,
    geometry: SamplingGeometry,
    (intersection, shadow, shadow_intersection, effects): (
        Option<CanvasIntersection>,
        Option<crate::DropShadow>,
        Option<CanvasIntersection>,
        Option<effects_tile::EffectsTileBounds>,
    ),
    stats: &mut RegionRenderStats,
) -> Result<bool> {
    if let (Some(shadow), Some(intersection)) = (shadow, shadow_intersection) {
        let alpha_tile =
            ShadowAlphaTile::bounded(&source, &geometry, base_layer, intersection, shadow);
        if let Some(tile) = &alpha_tile {
            let pixels = tile.pixel_count();
            stats.shadow_source_samples = stats.shadow_source_samples.saturating_add(pixels);
            stats.shadow_alpha_tile_pixels = stats.shadow_alpha_tile_pixels.saturating_add(pixels);
            stats.shadow_alpha_tile_bytes = stats.shadow_alpha_tile_bytes.saturating_add(pixels);
            stats.max_shadow_alpha_tile_pixels = stats.max_shadow_alpha_tile_pixels.max(pixels);
            stats.max_shadow_alpha_tile_bytes = stats.max_shadow_alpha_tile_bytes.max(pixels);
        }
        let pixels = ((intersection.bottom - intersection.top)
            * (intersection.right - intersection.left)) as u64;
        let taps = if shadow.blur_radius < 0.5 {
            1
        } else {
            crate::effects::DROP_SHADOW_KERNEL_TAPS
        };
        stats.shadow_samples = stats.shadow_samples.saturating_add(pixels * taps);
        if alpha_tile.is_none() {
            stats.shadow_source_samples = stats.shadow_source_samples.saturating_add(pixels * taps);
        }
        let stride = region.width as usize * 4;
        let (source, alpha_tile) = (&source, &alpha_tile);
        canvas
            .par_chunks_mut(stride)
            .enumerate()
            .skip((intersection.top - i64::from(region.y)) as usize)
            .take((intersection.bottom - intersection.top) as usize)
            .for_each(|(row, canvas_row)| {
                let canvas_y = i64::from(region.y) + row as i64;
                for canvas_x in intersection.left..intersection.right {
                    let center_x = canvas_x - geometry.origin_x - shadow.offset_x.round() as i64;
                    let center_y = canvas_y - geometry.origin_y - shadow.offset_y.round() as i64;
                    let alpha = alpha_tile.as_ref().map_or_else(
                        || {
                            drop_shadow_alpha(center_x, center_y, shadow.blur_radius, |x, y| {
                                sample_output_alpha(source, &geometry, base_layer, x, y)
                            })
                        },
                        |tile| tile.filtered_alpha(center_x, center_y, shadow.blur_radius),
                    );
                    let x = (canvas_x - i64::from(region.x)) as usize;
                    let pixel = colored_shadow_pixel(shadow, alpha);
                    let clip_alpha = if scaled_layer.clip_to_below {
                        clip.map_or(0.0, |image| {
                            f32::from(image.get_pixel(x as u32, row as u32)[3]) / 255.0
                        })
                    } else {
                        1.0
                    };
                    let strength = f32::from(pixel[3]) / 255.0 * scaled_layer.opacity * clip_alpha;
                    if strength > 0.0 {
                        let slot = &mut canvas_row[x * 4..x * 4 + 4];
                        let destination = [slot[0], slot[1], slot[2], slot[3]];
                        slot.copy_from_slice(&blend_pixel(
                            destination,
                            pixel,
                            crate::BlendMode::Normal,
                            strength,
                        ));
                    }
                }
            });
    }

    let effects = effects.map(|bounds| {
        let tile = bounds.sample(&source, &geometry, base_layer);
        let passes = crate::layer_effects::effect_passes(&scaled_layer.style, &tile);
        (tile, passes)
    });
    let origin = (geometry.origin_x, geometry.origin_y);
    if let Some((tile, (behind, _))) = &effects {
        composite_effect_passes(canvas, behind, tile, origin, scaled_layer, clip, region);
    }
    if let Some(intersection) = intersection {
        // Rows are independent: each samples the source and blends its own
        // canvas and coverage pixels.
        let stride = region.width as usize * 4;
        let first = (intersection.top - i64::from(region.y)) as usize;
        let rows = (intersection.bottom - intersection.top) as usize;
        let source = &source;
        canvas
            .par_chunks_mut(stride)
            .zip(coverage.par_chunks_mut(stride))
            .enumerate()
            .skip(first)
            .take(rows)
            .for_each(|(row, (canvas_row, coverage_row))| {
                let canvas_y = i64::from(region.y) + row as i64;
                for canvas_x in intersection.left..intersection.right {
                    let output_x = (canvas_x - geometry.origin_x) as u32;
                    let output_y = (canvas_y - geometry.origin_y) as u32;
                    let Some((scaled_x, scaled_y)) = geometry.inverse_sample(output_x, output_y)
                    else {
                        continue;
                    };
                    let source_pixel = sample_triangle_resize(
                        source,
                        (geometry.source_width, geometry.source_height),
                        (geometry.scaled_width, geometry.scaled_height),
                        (scaled_x, scaled_y),
                    );
                    let x = (canvas_x - i64::from(region.x)) as usize;
                    let alpha = layer_pixel_alpha(
                        source_pixel,
                        [
                            output_x,
                            output_y,
                            geometry.output_width,
                            geometry.output_height,
                        ],
                        scaled_layer,
                        clip,
                        [x as u32, row as u32, canvas_x as u32, canvas_y as u32],
                    );
                    if alpha <= 0.0 {
                        continue;
                    }
                    let slot = &mut canvas_row[x * 4..x * 4 + 4];
                    let destination = [slot[0], slot[1], slot[2], slot[3]];
                    slot.copy_from_slice(&blend_pixel(
                        destination,
                        source_pixel,
                        scaled_layer.blend_mode,
                        alpha,
                    ));
                    coverage_row[x * 4..x * 4 + 4].copy_from_slice(&[
                        255,
                        255,
                        255,
                        (alpha * 255.0) as u8,
                    ]);
                }
            });
    }
    if let Some((tile, (_, above))) = &effects {
        composite_effect_passes(canvas, above, tile, origin, scaled_layer, clip, region);
    }
    Ok(true)
}
