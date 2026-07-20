use std::collections::HashMap;

use anyhow::Result;
use image::RgbaImage;

use crate::{
    Layer, LayerKind, RenderRegion,
    render::{composite_pixel, render_layer_preview_scaled, render_solid_color},
    shapes::ShapeSampler,
};

/// Composites only the transformed pixels intersecting `region`.
///
/// This mirrors the export compositor's Triangle resize followed by its
/// nearest-neighbour rotation, but never materializes the transformed layer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn composite_layer_viewport(
    canvas: &mut RgbaImage,
    coverage: &mut RgbaImage,
    render_layer: &Layer,
    scaled_layer: &Layer,
    shape_scale: [f32; 2],
    clip: Option<&RgbaImage>,
    region: RenderRegion,
    stats: &mut crate::RegionRenderStats,
) -> Result<()> {
    let mut source = LayerSource::new(render_layer, shape_scale)?;
    let (source_width, source_height) = source.dimensions();
    stats.source_staging_pixels = stats
        .source_staging_pixels
        .saturating_add(source.source_staging_pixels());
    let scaled_width = scaled_dimension(source_width, scaled_layer.transform.scale_x);
    let scaled_height = scaled_dimension(source_height, scaled_layer.transform.scale_y);
    let rotation = scaled_layer.transform.rotation;
    let (output_width, output_height) = rotated_dimensions(scaled_width, scaled_height, rotation);
    let origin_x = scaled_layer.transform.x.round() as i64;
    let origin_y = scaled_layer.transform.y.round() as i64;
    let left = origin_x.max(i64::from(region.x));
    let top = origin_y.max(i64::from(region.y));
    let right = (origin_x + i64::from(output_width)).min(i64::from(region.x + region.width));
    let bottom = (origin_y + i64::from(output_height)).min(i64::from(region.y + region.height));
    if right <= left || bottom <= top {
        return Ok(());
    }

    for canvas_y in top..bottom {
        for canvas_x in left..right {
            let output_x = (canvas_x - origin_x) as u32;
            let output_y = (canvas_y - origin_y) as u32;
            let Some((scaled_x, scaled_y)) = inverse_rotation_sample(
                output_x,
                output_y,
                output_width,
                output_height,
                scaled_width,
                scaled_height,
                rotation,
            ) else {
                continue;
            };
            let source_pixel = sample_triangle_resize(
                &mut source,
                scaled_x,
                scaled_y,
                scaled_width,
                scaled_height,
            )?;
            composite_pixel(
                canvas,
                coverage,
                source_pixel,
                output_x,
                output_y,
                output_width,
                output_height,
                scaled_layer,
                clip,
                canvas_x as u32 - region.x,
                canvas_y as u32 - region.y,
            );
        }
    }
    stats.max_adjusted_tile_pixels = stats
        .max_adjusted_tile_pixels
        .max(source.max_adjusted_tile_pixels());
    Ok(())
}

enum LayerSource<'a> {
    Image(RgbaImage),
    Shape {
        sampler: ShapeSampler<'a>,
        adjustments: &'a spectrum_imaging::Adjustments,
        dimensions: (u32, u32),
        adjusted_pixels: HashMap<[u8; 4], [u8; 4]>,
        adjusted_tiles: HashMap<(u32, u32), RgbaImage>,
    },
}

impl<'a> LayerSource<'a> {
    fn new(layer: &'a Layer, shape_scale: [f32; 2]) -> Result<Self> {
        if matches!(
            layer.kind,
            LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. }
        ) {
            let sampler = ShapeSampler::new(layer, shape_scale)?;
            let source_dimensions = sampler.dimensions();
            return Ok(Self::Shape {
                dimensions: spectrum_imaging::adjusted_image_dimensions(
                    source_dimensions.0,
                    source_dimensions.1,
                    &layer.adjustments,
                ),
                sampler,
                adjustments: &layer.adjustments,
                adjusted_pixels: HashMap::new(),
                adjusted_tiles: HashMap::new(),
            });
        }
        Ok(Self::Image(
            render_layer_preview_scaled(layer, None, shape_scale)?.to_rgba8(),
        ))
    }

    fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Image(image) => image.dimensions(),
            Self::Shape { dimensions, .. } => *dimensions,
        }
    }

    fn pixel(&mut self, x: u32, y: u32) -> Result<[u8; 4]> {
        match self {
            Self::Image(image) => Ok(image.get_pixel(x, y).0),
            Self::Shape {
                sampler,
                adjustments,
                dimensions,
                adjusted_pixels,
                adjusted_tiles,
            } => {
                if !adjustments_are_point_local(adjustments) {
                    const TILE_SIZE: u32 = 64;
                    let tile_x = x / TILE_SIZE;
                    let tile_y = y / TILE_SIZE;
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        adjusted_tiles.entry((tile_x, tile_y))
                    {
                        let region = spectrum_imaging::PixelRegion {
                            x: tile_x * TILE_SIZE,
                            y: tile_y * TILE_SIZE,
                            width: TILE_SIZE.min(dimensions.0 - tile_x * TILE_SIZE),
                            height: TILE_SIZE.min(dimensions.1 - tile_y * TILE_SIZE),
                        };
                        let tile = spectrum_imaging::render_image_region(
                            sampler.dimensions().0,
                            sampler.dimensions().1,
                            (*adjustments).clone(),
                            region,
                            |source_x, source_y| sampler.pixel(source_x, source_y),
                        )
                        .map_err(anyhow::Error::msg)?;
                        entry.insert(tile);
                    }
                    return Ok(adjusted_tiles[&(tile_x, tile_y)]
                        .get_pixel(x % TILE_SIZE, y % TILE_SIZE)
                        .0);
                }
                let pixel = sampler.pixel(x, y).0;
                if adjustments.is_identity() {
                    Ok(pixel)
                } else if let Some(adjusted) = adjusted_pixels.get(&pixel) {
                    Ok(*adjusted)
                } else {
                    let adjusted = render_solid_color(pixel, adjustments);
                    adjusted_pixels.insert(pixel, adjusted);
                    Ok(adjusted)
                }
            }
        }
    }

    fn source_staging_pixels(&self) -> u64 {
        match self {
            Self::Image(image) => u64::from(image.width()) * u64::from(image.height()),
            Self::Shape { .. } => 0,
        }
    }

    fn max_adjusted_tile_pixels(&self) -> u64 {
        match self {
            Self::Image(_) => 0,
            Self::Shape { adjusted_tiles, .. } => adjusted_tiles
                .values()
                .map(|tile| u64::from(tile.width()) * u64::from(tile.height()))
                .max()
                .unwrap_or(0),
        }
    }
}

fn adjustments_are_point_local(adjustments: &spectrum_imaging::Adjustments) -> bool {
    adjustments.rotation == 0
        && !adjustments.flip_horizontal
        && !adjustments.flip_vertical
        && adjustments.straighten.abs() <= 0.01
        && adjustments.crop.is_none()
        && adjustments.noise_reduction <= 0.0
        && adjustments.sharpening <= 0.0
        && adjustments.vignette == 0.0
        && adjustments.spots.is_empty()
}

fn scaled_dimension(value: u32, scale: f32) -> u32 {
    (value as f32 * scale).round().max(1.0) as u32
}

fn rotated_dimensions(width: u32, height: u32, degrees: f32) -> (u32, u32) {
    if degrees.abs() < 0.01 {
        return (width, height);
    }
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    (
        (width as f32 * cos.abs() + height as f32 * sin.abs())
            .ceil()
            .max(1.0) as u32,
        (width as f32 * sin.abs() + height as f32 * cos.abs())
            .ceil()
            .max(1.0) as u32,
    )
}

#[allow(clippy::too_many_arguments)]
fn inverse_rotation_sample(
    output_x: u32,
    output_y: u32,
    output_width: u32,
    output_height: u32,
    source_width: u32,
    source_height: u32,
    degrees: f32,
) -> Option<(u32, u32)> {
    if degrees.abs() < 0.01 {
        return Some((output_x, output_y));
    }
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let source_center = (
        (source_width as f32 - 1.0) * 0.5,
        (source_height as f32 - 1.0) * 0.5,
    );
    let output_center = (
        (output_width - 1) as f32 * 0.5,
        (output_height - 1) as f32 * 0.5,
    );
    let dx = output_x as f32 - output_center.0;
    let dy = output_y as f32 - output_center.1;
    let source_x = cos * dx + sin * dy + source_center.0;
    let source_y = -sin * dx + cos * dy + source_center.1;
    if source_x < 0.0
        || source_y < 0.0
        || source_x >= source_width as f32
        || source_y >= source_height as f32
    {
        return None;
    }
    Some((
        source_x.round().clamp(0.0, source_width as f32 - 1.0) as u32,
        source_y.round().clamp(0.0, source_height as f32 - 1.0) as u32,
    ))
}

fn sample_triangle_resize(
    source: &mut LayerSource<'_>,
    output_x: u32,
    output_y: u32,
    output_width: u32,
    output_height: u32,
) -> Result<[u8; 4]> {
    let (source_width, source_height) = source.dimensions();
    if (source_width, source_height) == (output_width, output_height) {
        return source.pixel(output_x, output_y);
    }
    let x_weights = triangle_weights(source_width, output_width, output_x);
    let y_weights = triangle_weights(source_height, output_height, output_y);
    let mut horizontal = [0.0_f32; 4];
    for source_x in x_weights.start..x_weights.end {
        let mut vertical = [0.0_f32; 4];
        for source_y in y_weights.start..y_weights.end {
            let pixel = source.pixel(source_x, source_y)?;
            let weight =
                triangle_weight(source_y, y_weights.center, y_weights.scale) / y_weights.sum;
            for channel in 0..4 {
                vertical[channel] += f32::from(pixel[channel]) * weight;
            }
        }
        let weight = triangle_weight(source_x, x_weights.center, x_weights.scale) / x_weights.sum;
        for channel in 0..4 {
            horizontal[channel] += vertical[channel] * weight;
        }
    }
    Ok(horizontal.map(|channel| channel.round().clamp(0.0, 255.0) as u8))
}

struct TriangleWeights {
    start: u32,
    end: u32,
    center: f32,
    scale: f32,
    sum: f32,
}

fn triangle_weights(source: u32, output: u32, coordinate: u32) -> TriangleWeights {
    let ratio = source as f32 / output as f32;
    let scale = ratio.max(1.0);
    let input = (coordinate as f32 + 0.5) * ratio;
    let start = ((input - scale).floor() as i64).clamp(0, i64::from(source) - 1) as u32;
    let end = ((input + scale).ceil() as i64).clamp(i64::from(start) + 1, i64::from(source)) as u32;
    let center = input - 0.5;
    let sum = (start..end)
        .map(|sample| triangle_weight(sample, center, scale))
        .sum();
    TriangleWeights {
        start,
        end,
        center,
        scale,
        sum,
    }
}

fn triangle_weight(sample: u32, center: f32, scale: f32) -> f32 {
    (1.0 - ((sample as f32 - center) / scale).abs()).max(0.0)
}
