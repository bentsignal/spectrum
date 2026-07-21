use image::RgbaImage;

use crate::{
    DropShadow, Layer,
    effects::{DROP_SHADOW_KERNEL_TAPS, colored_shadow_pixel, drop_shadow_alpha},
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
    let radius = shadow.blur_radius.ceil() as i64;
    let left = (origin_x - radius).max(i64::from(region.x));
    let top = (origin_y - radius).max(i64::from(region.y));
    let right =
        (origin_x + i64::from(source.width()) + radius).min(i64::from(region.x + region.width));
    let bottom =
        (origin_y + i64::from(source.height()) + radius).min(i64::from(region.y + region.height));
    for canvas_y in top..bottom {
        for canvas_x in left..right {
            let center_x = canvas_x - origin_x;
            let center_y = canvas_y - origin_y;
            let alpha = drop_shadow_alpha(center_x, center_y, shadow.blur_radius, |x, y| {
                masked_source_alpha(source, layer, x, y)
            });
            stats.shadow_samples =
                stats
                    .shadow_samples
                    .saturating_add(if shadow.blur_radius < 0.5 {
                        1
                    } else {
                        DROP_SHADOW_KERNEL_TAPS
                    });
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
