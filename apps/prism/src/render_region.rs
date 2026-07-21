use anyhow::{Result, bail};
use image::RgbaImage;

use crate::{
    Document, FontAsset, Layer, LayerKind, RasterSourceResolver, RegionRenderStats, RenderRegion,
    render::composite_pixel, shapes::constrained_shape_scale,
    text_render::measure_text_geometry_with_typography,
};

mod source;
use source::{SampleSource, SourceDescriptor, SourceRegion};

const MAX_SOURCE_STAGING_PIXELS: u64 = 4_096 * 4_096;

/// Rasterization and outer-transform scales used by region-native sampling.
///
/// This is public so allocation-contract benchmarks can derive independent
/// source-space bounds from the exact production scale selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionSourceScales {
    pub text_raster: f32,
    pub shape_raster: [f32; 2],
    pub outer_transform: [f32; 2],
}

pub fn region_source_scales(
    document: &Document,
    layer: &Layer,
    document_scale: f32,
) -> Result<RegionSourceScales> {
    if !document_scale.is_finite() || document_scale <= 0.0 {
        bail!("document render scale must be a positive finite number");
    }
    let text_raster = text_raster_scale(layer, document_scale);
    let shape_raster = if matches!(
        layer.kind,
        LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. }
    ) {
        constrained_shape_scale(
            layer,
            [
                (layer.transform.scale_x.abs() * document_scale).max(1.0),
                (layer.transform.scale_y.abs() * document_scale).max(1.0),
            ],
            document.width.max(document.height),
        )?
    } else {
        [1.0; 2]
    };
    Ok(RegionSourceScales {
        text_raster,
        shape_raster,
        outer_transform: [
            layer.transform.scale_x * document_scale / text_raster / shape_raster[0],
            layer.transform.scale_y * document_scale / text_raster / shape_raster[1],
        ],
    })
}

fn text_raster_scale(layer: &Layer, document_scale: f32) -> f32 {
    if !matches!(layer.kind, LayerKind::Text { .. }) {
        return 1.0;
    }
    let target = layer
        .transform
        .scale_x
        .abs()
        .max(layer.transform.scale_y.abs())
        * document_scale;
    (target.max(1.0).ceil() as u32).next_power_of_two().min(16) as f32
}

pub(crate) fn supports_bounded_source(
    layer: &Layer,
    raster_sources: Option<&dyn RasterSourceResolver>,
) -> bool {
    matches!(
        layer.kind,
        LayerKind::Raster { .. }
            | LayerKind::Text { .. }
            | LayerKind::Rectangle { .. }
            | LayerKind::Ellipse { .. }
    ) && source::layer_supports_region_reads(layer, raster_sources)
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
    shape_scale: [f32; 2],
    clip: Option<&RgbaImage>,
    region: RenderRegion,
    font_asset: Option<&FontAsset>,
    raster_sources: Option<&dyn RasterSourceResolver>,
    stats: &mut RegionRenderStats,
) -> Result<bool> {
    if !supports_bounded_source(render_layer, raster_sources) {
        return Ok(false);
    }
    let descriptor = SourceDescriptor::new(render_layer, shape_scale, font_asset, raster_sources)?;
    let geometry = SamplingGeometry::new(
        base_layer,
        render_layer,
        scaled_layer,
        &descriptor,
        font_asset,
    )?;
    let Some(intersection) = geometry.intersection(region) else {
        return Ok(true);
    };
    let Some(staging_region) = required_source_region(&geometry, intersection) else {
        return Ok(true);
    };
    if staging_region.pixel_count() > MAX_SOURCE_STAGING_PIXELS {
        bail!(
            "layer {} requires more than the bounded source staging budget",
            render_layer.id
        );
    }
    let source = descriptor.sample(staging_region, stats)?;
    stats.full_source_pixels = stats
        .full_source_pixels
        .saturating_add(u64::from(geometry.source_width) * u64::from(geometry.source_height));

    for canvas_y in intersection.top..intersection.bottom {
        for canvas_x in intersection.left..intersection.right {
            let output_x = (canvas_x - geometry.origin_x) as u32;
            let output_y = (canvas_y - geometry.origin_y) as u32;
            let Some((scaled_x, scaled_y)) = geometry.inverse_sample(output_x, output_y) else {
                continue;
            };
            let source_pixel = sample_triangle_resize(
                &source,
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
        font_asset: Option<&FontAsset>,
    ) -> Result<Self> {
        let (source_width, source_height) = descriptor.dimensions()?;
        let scaled_width = scaled_dimension(source_width, scaled_layer.transform.scale_x);
        let scaled_height = scaled_dimension(source_height, scaled_layer.transform.scale_y);
        let degrees = scaled_layer.transform.rotation;
        let (output_width, output_height, offset, rotation) = match &render_layer.kind {
            LayerKind::Text { font_size, .. } if degrees.abs() >= 0.01 => {
                let LayerKind::Text {
                    text,
                    font_size: base_font_size,
                    typography,
                    ..
                } = &base_layer.kind
                else {
                    unreachable!("render layer mirrors its base layer")
                };
                let base_pivot = measure_text_geometry_with_typography(
                    text,
                    *base_font_size,
                    typography,
                    font_asset,
                )?
                .visual_center();
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
            LayerKind::Raster { .. } | LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. }
                if degrees.abs() >= 0.01 =>
            {
                let bounds = centered_rotation_bounds(scaled_width, scaled_height, degrees);
                (
                    bounds.width,
                    bounds.height,
                    (bounds.offset_x, bounds.offset_y),
                    RotationSampling::Center { degrees },
                )
            }
            LayerKind::Raster { .. } | LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } => (
                scaled_width,
                scaled_height,
                (0.0, 0.0),
                RotationSampling::None,
            ),
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

fn sample_triangle_resize(
    source_pixels: &SampleSource<'_>,
    source: (u32, u32),
    output: (u32, u32),
    coordinate: (u32, u32),
) -> [u8; 4] {
    if let SampleSource::Constant(pixel) = source_pixels {
        return *pixel;
    }
    if source == output {
        return source_pixels.pixel(coordinate.0, coordinate.1);
    }
    let x_weights = triangle_weights(source.0, output.0, coordinate.0);
    let y_weights = triangle_weights(source.1, output.1, coordinate.1);
    let mut horizontal = [0.0_f32; 4];
    for source_x in x_weights.start..x_weights.end {
        let mut vertical = [0.0_f32; 4];
        for source_y in y_weights.start..y_weights.end {
            let pixel = source_pixels.pixel(source_x, source_y);
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CenteredRotationBounds {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) offset_x: f32,
    pub(crate) offset_y: f32,
}

pub(crate) fn centered_rotation_bounds(
    width: u32,
    height: u32,
    degrees: f32,
) -> CenteredRotationBounds {
    let (sin, cos) = crate::transform_math::rotation_sin_cos(degrees);
    let output_width = (width as f32 * cos.abs() + height as f32 * sin.abs())
        .ceil()
        .max(1.0) as u32;
    let output_height = (width as f32 * sin.abs() + height as f32 * cos.abs())
        .ceil()
        .max(1.0) as u32;
    CenteredRotationBounds {
        width: output_width,
        height: output_height,
        offset_x: (width as f32 - output_width as f32) * 0.5,
        offset_y: (height as f32 - output_height as f32) * 0.5,
    }
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
    let (sin, cos) = crate::transform_math::rotation_sin_cos(degrees);
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
    let (sin, cos) = crate::transform_math::rotation_sin_cos(degrees);
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
    let (sin, cos) = crate::transform_math::rotation_sin_cos(degrees);
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
    use crate::{Layer, LayerKind, ShapeStroke, shapes::ShapeSampler};

    #[test]
    fn uniform_rectangles_bypass_procedural_and_triangle_sampling() {
        let color = [37, 91, 173, 211];
        let layer = Layer {
            kind: LayerKind::Rectangle {
                width: 16_384,
                height: 16_384,
                color,
                corner_radius: 0.0,
            },
            ..Layer::default()
        };
        let source = SampleSource::shape(ShapeSampler::new(&layer, [1.0; 2]).unwrap());
        assert!(matches!(&source, SampleSource::Constant(pixel) if *pixel == color));
        assert_eq!(
            sample_triangle_resize(
                &source,
                (16_384, 16_384),
                (131_072, 131_072),
                (79_123, 62_417),
            ),
            color
        );

        for layer in [
            Layer {
                kind: LayerKind::Rectangle {
                    width: 128,
                    height: 96,
                    color,
                    corner_radius: 8.0,
                },
                ..Layer::default()
            },
            Layer {
                stroke: ShapeStroke {
                    enabled: true,
                    width: 3.0,
                    color: [240, 220, 180, 255],
                },
                kind: LayerKind::Rectangle {
                    width: 128,
                    height: 96,
                    color,
                    corner_radius: 0.0,
                },
                ..Layer::default()
            },
        ] {
            assert!(matches!(
                SampleSource::shape(ShapeSampler::new(&layer, [1.0; 2]).unwrap()),
                SampleSource::Shape(_)
            ));
        }
    }

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

    #[test]
    fn centered_rotation_bounds_keep_the_source_center_fixed() {
        for degrees in [17.0, 45.0, 90.0, 137.0, 270.0] {
            let bounds = centered_rotation_bounds(100, 40, degrees);
            let source_center = (50.0, 20.0);
            let output_center = (
                bounds.offset_x + bounds.width as f32 * 0.5,
                bounds.offset_y + bounds.height as f32 * 0.5,
            );

            assert!((output_center.0 - source_center.0).abs() < 0.001);
            assert!((output_center.1 - source_center.1).abs() < 0.001);
        }
        let right_angle = centered_rotation_bounds(100, 40, 90.0);
        assert_eq!((right_angle.width, right_angle.height), (40, 100));
    }
}
