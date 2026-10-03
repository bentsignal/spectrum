use image::RgbaImage;

use crate::{
    DropShadow, Layer,
    effects::{blur_shadow_alpha, colored_shadow_pixel, shadow_reach},
    layer_effects::{AlphaTile, EffectPass},
    render::{RegionRenderStats, RenderRegion, composite_blended_pixel, layer_mask_allows},
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn composite_shadow_region(
    canvas: &mut RgbaImage,
    source: &RgbaImage,
    layer: &Layer,
    clip: Option<&RgbaImage>,
    region: RenderRegion,
    shadow: DropShadow,
    stats: &mut RegionRenderStats,
) {
    let origin_x = layer.transform.x.round() as i64 + shadow.offset_x.round() as i64;
    let origin_y = layer.transform.y.round() as i64 + shadow.offset_y.round() as i64;
    // The shadow is the layer's alpha blurred once into a tile padded by how
    // far the blur reaches, then looked up per pixel.
    let reach = shadow_reach(shadow.blur_radius);
    let tile_width = source.width() as usize + 2 * reach as usize;
    let tile_height = source.height() as usize + 2 * reach as usize;
    let mut tile = Vec::with_capacity(tile_width * tile_height);
    for y in 0..tile_height as i64 {
        for x in 0..tile_width as i64 {
            tile.push(masked_source_alpha(source, layer, x - reach, y - reach));
        }
    }
    blur_shadow_alpha(&mut tile, tile_width, tile_height, shadow.blur_radius);
    let left = (origin_x - reach).max(i64::from(region.x));
    let top = (origin_y - reach).max(i64::from(region.y));
    let right =
        (origin_x + i64::from(source.width()) + reach).min(i64::from(region.x + region.width));
    let bottom =
        (origin_y + i64::from(source.height()) + reach).min(i64::from(region.y + region.height));
    for canvas_y in top..bottom {
        for canvas_x in left..right {
            let tile_x = (canvas_x - origin_x + reach) as usize;
            let tile_y = (canvas_y - origin_y + reach) as usize;
            let alpha = tile[tile_y * tile_width + tile_x];
            stats.shadow_samples = stats.shadow_samples.saturating_add(1);
            composite_style_pixel(
                canvas,
                colored_shadow_pixel(shadow, alpha),
                layer.opacity,
                layer.clip_to_below,
                clip,
                canvas_x as u32 - region.x,
                canvas_y as u32 - region.y,
            );
        }
    }
}

fn masked_source_alpha(source: &RgbaImage, layer: &Layer, x: i64, y: i64) -> u8 {
    if x < 0 || y < 0 || x >= i64::from(source.width()) || y >= i64::from(source.height()) {
        return 0;
    }
    if !layer_mask_allows(layer, x as u32, y as u32, source.width(), source.height()) {
        0
    } else {
        source.get_pixel(x as u32, y as u32)[3]
    }
}

pub(crate) fn composite_style_pixel(
    canvas: &mut RgbaImage,
    source_pixel: [u8; 4],
    opacity: f32,
    clipped: bool,
    clip: Option<&RgbaImage>,
    x: u32,
    y: u32,
) {
    let clip_alpha = if clipped {
        clip.map_or(0.0, |image| image.get_pixel(x, y)[3] as f32 / 255.0)
    } else {
        1.0
    };
    let alpha = source_pixel[3] as f32 / 255.0 * opacity * clip_alpha;
    if alpha > 0.0 {
        composite_blended_pixel(canvas, source_pixel, crate::BlendMode::Normal, alpha, x, y);
    }
}

/// The layer's alpha around its source image, padded by `reach` so styles
/// can spread past its edge.
pub(crate) fn source_alpha_tile(source: &RgbaImage, layer: &Layer, reach: i64) -> AlphaTile {
    let width = source.width() as usize + 2 * reach as usize;
    let height = source.height() as usize + 2 * reach as usize;
    let mut alpha = vec![0u8; width * height];
    {
        use rayon::prelude::*;
        alpha
            .par_chunks_mut(width.max(1))
            .enumerate()
            .for_each(|(y, row)| {
                for (x, value) in row.iter_mut().enumerate() {
                    *value = masked_source_alpha(source, layer, x as i64 - reach, y as i64 - reach);
                }
            });
    }
    AlphaTile {
        left: -reach,
        top: -reach,
        width,
        height,
        alpha,
        extent: [source.width() as f32, source.height() as f32],
    }
}

/// Draws style passes computed over `tile`, whose pixel (0, 0) sits at
/// `origin + (tile.left, tile.top)` on the canvas, within `region`. Rows
/// blend in parallel; each pixel takes the passes in order.
pub(crate) fn composite_effect_passes(
    canvas: &mut RgbaImage,
    passes: &[EffectPass],
    tile: &AlphaTile,
    origin: (i64, i64),
    layer: &Layer,
    clip: Option<&RgbaImage>,
    region: RenderRegion,
) {
    use rayon::prelude::*;
    if passes.is_empty() {
        return;
    }
    let left = origin.0 + tile.left;
    let top = origin.1 + tile.top;
    let x0 = left.max(i64::from(region.x));
    let y0 = top.max(i64::from(region.y));
    let x1 = (left + tile.width as i64).min(i64::from(region.x + region.width));
    let y1 = (top + tile.height as i64).min(i64::from(region.y + region.height));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let stride = region.width as usize * 4;
    let (opacity, clipped) = (layer.opacity, layer.clip_to_below);
    canvas
        .par_chunks_mut(stride)
        .enumerate()
        .skip((y0 - i64::from(region.y)) as usize)
        .take((y1 - y0) as usize)
        .for_each(|(row, pixels)| {
            let canvas_y = i64::from(region.y) + row as i64;
            let tile_row = (canvas_y - top) as usize * tile.width;
            for canvas_x in x0..x1 {
                let x = (canvas_x - i64::from(region.x)) as usize;
                let clip_alpha = if clipped {
                    clip.map_or(0.0, |image| {
                        f32::from(image.get_pixel(x as u32, row as u32)[3]) / 255.0
                    })
                } else {
                    1.0
                };
                if clip_alpha <= 0.0 {
                    continue;
                }
                let at = tile_row + (canvas_x - left) as usize;
                let slot = &mut pixels[x * 4..x * 4 + 4];
                for pass in passes {
                    let strength = pass.alpha[at];
                    if strength == 0 {
                        continue;
                    }
                    let color = pass.colors.as_ref().map_or(pass.color, |colors| {
                        let [r, g, b, a] = colors[at];
                        [
                            r,
                            g,
                            b,
                            (u16::from(a) * u16::from(pass.color[3]) / 255) as u8,
                        ]
                    });
                    let alpha = f32::from(strength) / 255.0 * f32::from(color[3]) / 255.0
                        * opacity
                        * clip_alpha;
                    if alpha <= 0.0 {
                        continue;
                    }
                    let source = [color[0], color[1], color[2], 255];
                    let destination = [slot[0], slot[1], slot[2], slot[3]];
                    slot.copy_from_slice(&crate::render::blend_pixel(
                        destination,
                        source,
                        pass.mode,
                        alpha,
                    ));
                }
            }
        });
}
