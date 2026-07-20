use std::{fs::File, io::BufReader, path::Path};

use anyhow::{Context, Result, bail};
use image::{ImageFormat, Rgba, RgbaImage};

use crate::{
    Layer, LayerKind, RegionRenderStats, RenderRegion,
    render::composite_pixel,
    text_render::{measure_text, measure_text_geometry, render_text_region},
};

const MAX_SOURCE_STAGING_PIXELS: u64 = 4_096 * 4_096;
const MAX_FALLBACK_DECODE_BYTES: u64 = 64 * 1_024 * 1_024;

pub(crate) fn supports_bounded_source(layer: &Layer) -> bool {
    matches!(
        layer.kind,
        LayerKind::Raster { .. } | LayerKind::Text { .. }
    ) && layer.adjustments == spectrum_imaging::Adjustments::default()
        && layer.transform.scale_x.is_finite()
        && layer.transform.scale_y.is_finite()
        && layer.transform.scale_x > 0.0
        && layer.transform.scale_y > 0.0
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn composite_bounded_source_region(
    canvas: &mut RgbaImage,
    coverage: &mut RgbaImage,
    base_layer: &Layer,
    render_layer: &Layer,
    scaled_layer: &Layer,
    clip: Option<&RgbaImage>,
    region: RenderRegion,
    stats: &mut RegionRenderStats,
) -> Result<bool> {
    if !supports_bounded_source(render_layer) {
        return Ok(false);
    }
    let descriptor = SourceDescriptor::new(base_layer, render_layer)?;
    let geometry = SamplingGeometry::new(base_layer, render_layer, scaled_layer, &descriptor)?;
    let Some(intersection) = geometry.intersection(region) else {
        return Ok(true);
    };
    let Some(staging_region) = required_source_region(&geometry, intersection) else {
        return Ok(true);
    };
    let staging_pixels = staging_region.pixel_count();
    if staging_pixels > MAX_SOURCE_STAGING_PIXELS {
        bail!(
            "layer {} requires more than the bounded source staging budget",
            render_layer.id
        );
    }
    let (staging, fallback_decode_bytes) = descriptor.stage(staging_region)?;
    stats.source_staging_pixels = stats.source_staging_pixels.saturating_add(staging_pixels);
    stats.source_staging_bytes = stats
        .source_staging_bytes
        .saturating_add(staging_pixels.saturating_mul(4));
    stats.max_source_staging_pixels = stats.max_source_staging_pixels.max(staging_pixels);
    stats.full_source_pixels = stats
        .full_source_pixels
        .saturating_add(u64::from(geometry.source_width) * u64::from(geometry.source_height));
    stats.fallback_decode_bytes = stats
        .fallback_decode_bytes
        .saturating_add(fallback_decode_bytes);

    for canvas_y in intersection.top..intersection.bottom {
        for canvas_x in intersection.left..intersection.right {
            let output_x = (canvas_x - geometry.origin_x) as u32;
            let output_y = (canvas_y - geometry.origin_y) as u32;
            let Some((scaled_x, scaled_y)) = geometry.inverse_sample(output_x, output_y) else {
                continue;
            };
            let source_pixel = sample_triangle_resize(
                &staging,
                staging_region,
                (geometry.source_width, geometry.source_height),
                (geometry.scaled_width, geometry.scaled_height),
                (scaled_x, scaled_y),
            );
            composite_pixel(
                canvas,
                coverage,
                source_pixel,
                output_x,
                output_y,
                geometry.output_width,
                geometry.output_height,
                scaled_layer,
                clip,
                canvas_x as u32 - region.x,
                canvas_y as u32 - region.y,
            );
        }
    }
    Ok(true)
}

enum SourceDescriptor<'a> {
    Raster {
        path: &'a Path,
        dimensions: (u32, u32),
    },
    Text {
        text: &'a str,
        font_size: f32,
        color: [u8; 4],
        dimensions: (u32, u32),
    },
}

impl<'a> SourceDescriptor<'a> {
    fn new(_base_layer: &'a Layer, render_layer: &'a Layer) -> Result<Self> {
        match &render_layer.kind {
            LayerKind::Raster { path, .. } => Ok(Self::Raster {
                dimensions: image::image_dimensions(path).with_context(|| {
                    format!("could not inspect layer source {}", path.display())
                })?,
                path,
            }),
            LayerKind::Text {
                text,
                font_size,
                color,
                ..
            } => Ok(Self::Text {
                text,
                font_size: *font_size,
                color: *color,
                dimensions: measure_text(text, *font_size)?,
            }),
            LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } => {
                unreachable!("bounded source descriptor only accepts raster and text layers")
            }
        }
    }

    fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Raster { dimensions, .. } | Self::Text { dimensions, .. } => *dimensions,
        }
    }

    fn stage(&self, region: SourceRegion) -> Result<(RgbaImage, u64)> {
        match self {
            Self::Raster { path, dimensions } => stage_raster(path, *dimensions, region),
            Self::Text {
                text,
                font_size,
                color,
                ..
            } => Ok((
                render_text_region(text, *font_size, *color, region.into())?,
                0,
            )),
        }
    }
}

#[derive(Clone, Copy)]
struct SamplingGeometry {
    source_width: u32,
    source_height: u32,
    scaled_width: u32,
    scaled_height: u32,
    output_width: u32,
    output_height: u32,
    origin_x: i64,
    origin_y: i64,
    rotation: RotationSampling,
}

#[derive(Clone, Copy)]
enum RotationSampling {
    None,
    Center {
        degrees: f32,
    },
    TextPivot {
        degrees: f32,
        pivot: (f32, f32),
        minimum: (f32, f32),
    },
}

impl SamplingGeometry {
    fn new(
        base_layer: &Layer,
        render_layer: &Layer,
        scaled_layer: &Layer,
        descriptor: &SourceDescriptor<'_>,
    ) -> Result<Self> {
        let (source_width, source_height) = descriptor.dimensions();
        let scaled_width = scaled_dimension(source_width, scaled_layer.transform.scale_x);
        let scaled_height = scaled_dimension(source_height, scaled_layer.transform.scale_y);
        let degrees = scaled_layer.transform.rotation;
        let (output_width, output_height, offset, rotation) = match &render_layer.kind {
            LayerKind::Text { font_size, .. } if degrees.abs() >= 0.01 => {
                let LayerKind::Text {
                    text,
                    font_size: base_font_size,
                    ..
                } = &base_layer.kind
                else {
                    unreachable!("render layer mirrors its base layer")
                };
                let base_pivot = measure_text_geometry(text, *base_font_size)?.visual_center();
                let font_ratio = *font_size / *base_font_size;
                let pivot = (
                    base_pivot.0 * scaled_layer.transform.scale_x * font_ratio,
                    base_pivot.1 * scaled_layer.transform.scale_y * font_ratio,
                );
                let bounds = rotated_text_bounds(scaled_width, scaled_height, degrees, pivot);
                (
                    bounds.width,
                    bounds.height,
                    (bounds.min_x, bounds.min_y),
                    RotationSampling::TextPivot {
                        degrees,
                        pivot,
                        minimum: (bounds.min_x, bounds.min_y),
                    },
                )
            }
            LayerKind::Text { .. } => (
                scaled_width,
                scaled_height,
                (0.0, 0.0),
                RotationSampling::None,
            ),
            LayerKind::Raster { .. } if degrees.abs() >= 0.01 => {
                let (width, height) = rotated_dimensions(scaled_width, scaled_height, degrees);
                (
                    width,
                    height,
                    (0.0, 0.0),
                    RotationSampling::Center { degrees },
                )
            }
            LayerKind::Raster { .. } => (
                scaled_width,
                scaled_height,
                (0.0, 0.0),
                RotationSampling::None,
            ),
            LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } => {
                unreachable!("bounded source geometry only accepts raster and text layers")
            }
        };
        Ok(Self {
            source_width,
            source_height,
            scaled_width,
            scaled_height,
            output_width,
            output_height,
            origin_x: (scaled_layer.transform.x + offset.0).round() as i64,
            origin_y: (scaled_layer.transform.y + offset.1).round() as i64,
            rotation,
        })
    }

    fn intersection(self, region: RenderRegion) -> Option<CanvasIntersection> {
        let left = self.origin_x.max(i64::from(region.x));
        let top = self.origin_y.max(i64::from(region.y));
        let right =
            (self.origin_x + i64::from(self.output_width)).min(i64::from(region.x + region.width));
        let bottom = (self.origin_y + i64::from(self.output_height))
            .min(i64::from(region.y + region.height));
        (right > left && bottom > top).then_some(CanvasIntersection {
            left,
            top,
            right,
            bottom,
        })
    }

    fn inverse_sample(self, output_x: u32, output_y: u32) -> Option<(u32, u32)> {
        match self.rotation {
            RotationSampling::None => Some((output_x, output_y)),
            RotationSampling::Center { degrees } => inverse_center_rotation(
                output_x,
                output_y,
                self.output_width,
                self.output_height,
                self.scaled_width,
                self.scaled_height,
                degrees,
            ),
            RotationSampling::TextPivot {
                degrees,
                pivot,
                minimum,
            } => inverse_text_rotation(
                output_x,
                output_y,
                degrees,
                pivot,
                minimum,
                (self.scaled_width, self.scaled_height),
            ),
        }
    }
}

#[derive(Clone, Copy)]
struct CanvasIntersection {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl SourceRegion {
    fn pixel_count(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

impl From<SourceRegion> for RenderRegion {
    fn from(region: SourceRegion) -> Self {
        Self {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        }
    }
}

fn required_source_region(
    geometry: &SamplingGeometry,
    intersection: CanvasIntersection,
) -> Option<SourceRegion> {
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for canvas_y in intersection.top..intersection.bottom {
        for canvas_x in intersection.left..intersection.right {
            let output_x = (canvas_x - geometry.origin_x) as u32;
            let output_y = (canvas_y - geometry.origin_y) as u32;
            let Some((scaled_x, scaled_y)) = geometry.inverse_sample(output_x, output_y) else {
                continue;
            };
            let x = source_sample_bounds(geometry.source_width, geometry.scaled_width, scaled_x);
            let y = source_sample_bounds(geometry.source_height, geometry.scaled_height, scaled_y);
            bounds = Some(match bounds {
                Some((left, top, right, bottom)) => (
                    left.min(x.start),
                    top.min(y.start),
                    right.max(x.end),
                    bottom.max(y.end),
                ),
                None => (x.start, y.start, x.end, y.end),
            });
        }
    }
    bounds.map(|(left, top, right, bottom)| SourceRegion {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

fn stage_raster(
    path: &Path,
    dimensions: (u32, u32),
    region: SourceRegion,
) -> Result<(RgbaImage, u64)> {
    let reader = image::ImageReader::open(path)
        .with_context(|| format!("could not open {}", path.display()))?
        .with_guessed_format()?;
    if reader.format() == Some(ImageFormat::Png)
        && let Some(image) = stage_png(path, dimensions, region)?
    {
        return Ok((image, 0));
    }
    let mut reader = reader;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_FALLBACK_DECODE_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .with_context(|| format!("could not decode bounded fallback {}", path.display()))?;
    let decoded_bytes = decoded.as_bytes().len() as u64;
    let rgba_bytes = u64::from(dimensions.0)
        .saturating_mul(u64::from(dimensions.1))
        .saturating_mul(4);
    if rgba_bytes > MAX_FALLBACK_DECODE_BYTES {
        bail!("raster requires more than the bounded full-source decode budget");
    }
    let rgba = decoded.to_rgba8();
    let staged = image::imageops::crop_imm(&rgba, region.x, region.y, region.width, region.height)
        .to_image();
    Ok((staged, decoded_bytes))
}

fn stage_png(
    path: &Path,
    dimensions: (u32, u32),
    region: SourceRegion,
) -> Result<Option<RgbaImage>> {
    let file = File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let mut decoder = png::Decoder::new_with_limits(
        BufReader::new(file),
        png::Limits {
            bytes: MAX_FALLBACK_DECODE_BYTES as usize,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .with_context(|| format!("could not decode PNG header {}", path.display()))?;
    if reader.info().interlaced {
        return Ok(None);
    }
    if (reader.info().width, reader.info().height) != dimensions {
        bail!("PNG dimensions changed while staging {}", path.display());
    }
    let (color_type, bit_depth) = reader.output_color_type();
    if bit_depth != png::BitDepth::Eight {
        bail!("PNG staging requires 8-bit transformed rows");
    }
    let channels = png_channels(color_type);
    let row_bytes = u64::from(dimensions.0) * channels as u64;
    if row_bytes > MAX_FALLBACK_DECODE_BYTES {
        bail!("PNG scanline exceeds the bounded source staging budget");
    }
    let mut output = RgbaImage::new(region.width, region.height);
    for source_y in 0..region.y + region.height {
        let row = reader
            .next_row()
            .with_context(|| format!("could not decode PNG row {}", path.display()))?
            .context("PNG ended before the requested source region")?;
        if source_y < region.y {
            continue;
        }
        for source_x in region.x..region.x + region.width {
            let offset = source_x as usize * channels;
            output.put_pixel(
                source_x - region.x,
                source_y - region.y,
                Rgba(png_pixel(
                    &row.data()[offset..offset + channels],
                    color_type,
                )),
            );
        }
    }
    Ok(Some(output))
}

fn png_channels(color_type: png::ColorType) -> usize {
    match color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => unreachable!("EXPAND removes indexed PNG output"),
    }
}

fn png_pixel(bytes: &[u8], color_type: png::ColorType) -> [u8; 4] {
    match color_type {
        png::ColorType::Grayscale => [bytes[0], bytes[0], bytes[0], 255],
        png::ColorType::GrayscaleAlpha => [bytes[0], bytes[0], bytes[0], bytes[1]],
        png::ColorType::Rgb => [bytes[0], bytes[1], bytes[2], 255],
        png::ColorType::Rgba => [bytes[0], bytes[1], bytes[2], bytes[3]],
        png::ColorType::Indexed => unreachable!("EXPAND removes indexed PNG output"),
    }
}

fn sample_triangle_resize(
    staging: &RgbaImage,
    staging_region: SourceRegion,
    source: (u32, u32),
    output: (u32, u32),
    coordinate: (u32, u32),
) -> [u8; 4] {
    if source == output {
        return staging
            .get_pixel(
                coordinate.0 - staging_region.x,
                coordinate.1 - staging_region.y,
            )
            .0;
    }
    let x_weights = triangle_weights(source.0, output.0, coordinate.0);
    let y_weights = triangle_weights(source.1, output.1, coordinate.1);
    let mut horizontal = [0.0_f32; 4];
    for source_x in x_weights.start..x_weights.end {
        let mut vertical = [0.0_f32; 4];
        for source_y in y_weights.start..y_weights.end {
            let pixel = staging.get_pixel(source_x - staging_region.x, source_y - staging_region.y);
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
    horizontal.map(|channel| channel.round().clamp(0.0, 255.0) as u8)
}

#[derive(Clone, Copy)]
struct TriangleWeights {
    start: u32,
    end: u32,
    center: f32,
    scale: f32,
    sum: f32,
}

fn source_sample_bounds(source: u32, output: u32, coordinate: u32) -> TriangleWeights {
    if source == output {
        return TriangleWeights {
            start: coordinate,
            end: coordinate + 1,
            center: coordinate as f32,
            scale: 1.0,
            sum: 1.0,
        };
    }
    triangle_weights(source, output, coordinate)
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

fn scaled_dimension(value: u32, scale: f32) -> u32 {
    (value as f32 * scale).round().max(1.0) as u32
}

fn rotated_dimensions(width: u32, height: u32, degrees: f32) -> (u32, u32) {
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

fn inverse_center_rotation(
    output_x: u32,
    output_y: u32,
    output_width: u32,
    output_height: u32,
    source_width: u32,
    source_height: u32,
    degrees: f32,
) -> Option<(u32, u32)> {
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
    rounded_source_sample(source_x, source_y, source_width, source_height)
}

fn inverse_text_rotation(
    output_x: u32,
    output_y: u32,
    degrees: f32,
    pivot: (f32, f32),
    minimum: (f32, f32),
    source: (u32, u32),
) -> Option<(u32, u32)> {
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let world_x = minimum.0 + output_x as f32 + 0.5;
    let world_y = minimum.1 + output_y as f32 + 0.5;
    let dx = world_x - pivot.0;
    let dy = world_y - pivot.1;
    let source_x = cos * dx + sin * dy + pivot.0 - 0.5;
    let source_y = -sin * dx + cos * dy + pivot.1 - 0.5;
    rounded_source_sample(source_x, source_y, source.0, source.1)
}

fn rounded_source_sample(x: f32, y: f32, width: u32, height: u32) -> Option<(u32, u32)> {
    if x < 0.0 || y < 0.0 || x >= width as f32 || y >= height as f32 {
        return None;
    }
    Some((
        x.round().clamp(0.0, width.saturating_sub(1) as f32) as u32,
        y.round().clamp(0.0, height.saturating_sub(1) as f32) as u32,
    ))
}

struct TextBounds {
    min_x: f32,
    min_y: f32,
    width: u32,
    height: u32,
}

fn rotated_text_bounds(width: u32, height: u32, degrees: f32, pivot: (f32, f32)) -> TextBounds {
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let rotate = |point: (f32, f32)| {
        let dx = point.0 - pivot.0;
        let dy = point.1 - pivot.1;
        (pivot.0 + dx * cos - dy * sin, pivot.1 + dx * sin + dy * cos)
    };
    let corners = [
        rotate((0.0, 0.0)),
        rotate((width as f32, 0.0)),
        rotate((width as f32, height as f32)),
        rotate((0.0, height as f32)),
    ];
    let min_x = (corners
        .iter()
        .map(|point| point.0)
        .fold(f32::INFINITY, f32::min)
        + 0.0001)
        .floor();
    let min_y = (corners
        .iter()
        .map(|point| point.1)
        .fold(f32::INFINITY, f32::min)
        + 0.0001)
        .floor();
    let max_x = (corners
        .iter()
        .map(|point| point.0)
        .fold(f32::NEG_INFINITY, f32::max)
        - 0.0001)
        .ceil();
    let max_y = (corners
        .iter()
        .map(|point| point.1)
        .fold(f32::NEG_INFINITY, f32::max)
        - 0.0001)
        .ceil();
    TextBounds {
        min_x,
        min_y,
        width: (max_x - min_x).max(1.0) as u32,
        height: (max_y - min_y).max(1.0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_bounds_include_triangle_filter_support() {
        let geometry = SamplingGeometry {
            source_width: 1_024,
            source_height: 768,
            scaled_width: 2_048,
            scaled_height: 1_536,
            output_width: 2_048,
            output_height: 1_536,
            origin_x: 0,
            origin_y: 0,
            rotation: RotationSampling::None,
        };
        let region = required_source_region(
            &geometry,
            CanvasIntersection {
                left: 500,
                top: 400,
                right: 820,
                bottom: 580,
            },
        )
        .unwrap();
        assert!(region.width < geometry.source_width);
        assert!(region.height < geometry.source_height);
        assert!(region.pixel_count() <= 320 * 180);
    }
}
