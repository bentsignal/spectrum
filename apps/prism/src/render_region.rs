use anyhow::{Context, Result, bail};
use image::RgbaImage;

use crate::{
    Document, FontAsset, Layer, LayerKind, RasterSourceResolver, RegionRenderStats, RenderRegion,
    effects::{DROP_SHADOW_KERNEL, colored_shadow_pixel, drop_shadow_alpha},
    effects_render::composite_style_pixel,
    render::composite_pixel,
    shapes::constrained_shape_scale,
    text_render::measure_text_geometry_with_typography,
};

mod source;
use source::{
    SampleSource, SourceDescriptor, SourceRegion, sample_triangle_resize,
    sample_triangle_resize_alpha, source_sample_bounds,
};
mod shadow_tile;
use shadow_tile::ShadowAlphaTile;

#[derive(Debug)]
pub(crate) struct SourceStagingBudgetExceeded;

impl std::fmt::Display for SourceStagingBudgetExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("layer exceeds the bounded source staging budget")
    }
}

impl std::error::Error for SourceStagingBudgetExceeded {}

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
    let text_raster = recommended_text_raster_scale(layer, document_scale);
    let shape_raster = if matches!(
        layer.kind,
        LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } | LayerKind::Path { .. }
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

/// Chooses the bounded power-of-two text source scale shared by viewport,
/// export, and interactive preview rendering.
pub fn recommended_text_raster_scale(layer: &Layer, document_scale: f32) -> f32 {
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
            | LayerKind::Path { .. }
            | LayerKind::Paint { .. }
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
    let intersection = geometry.intersection(region);
    let shadow = scaled_layer.style.drop_shadow;
    let shadow_intersection =
        shadow.and_then(|shadow| geometry.shadow_intersection(region, shadow));
    if intersection.is_none() && shadow_intersection.is_none() {
        return Ok(true);
    }
    let staging_region = if descriptor.is_unadjusted_shape() {
        SourceRegion {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }
    } else {
        let mut staging_region =
            intersection.and_then(|intersection| required_source_region(&geometry, intersection));
        if let (Some(shadow), Some(intersection)) = (shadow, shadow_intersection) {
            let shadow_region = required_shadow_source_region(&geometry, intersection, shadow);
            staging_region = union_source_regions(staging_region, shadow_region);
        }
        let Some(staging_region) = staging_region else {
            return Ok(true);
        };
        staging_region
    };
    if staging_region.pixel_count() > MAX_SOURCE_STAGING_PIXELS {
        return Err(anyhow::Error::new(SourceStagingBudgetExceeded))
            .with_context(|| format!("layer {} cannot stage its source region", render_layer.id));
    }
    let source = descriptor.sample(staging_region, stats)?;
    stats.full_source_pixels = stats
        .full_source_pixels
        .saturating_add(u64::from(geometry.source_width) * u64::from(geometry.source_height));

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
        for canvas_y in intersection.top..intersection.bottom {
            for canvas_x in intersection.left..intersection.right {
                let center_x = canvas_x - geometry.origin_x - shadow.offset_x.round() as i64;
                let center_y = canvas_y - geometry.origin_y - shadow.offset_y.round() as i64;
                let alpha = alpha_tile.as_ref().map_or_else(
                    || {
                        drop_shadow_alpha(center_x, center_y, shadow.blur_radius, |x, y| {
                            sample_output_alpha(&source, &geometry, base_layer, x, y)
                        })
                    },
                    |tile| tile.filtered_alpha(center_x, center_y, shadow.blur_radius),
                );
                stats.shadow_samples =
                    stats
                        .shadow_samples
                        .saturating_add(if shadow.blur_radius < 0.5 {
                            1
                        } else {
                            crate::effects::DROP_SHADOW_KERNEL_TAPS
                        });
                if alpha_tile.is_none() {
                    stats.shadow_source_samples =
                        stats
                            .shadow_source_samples
                            .saturating_add(if shadow.blur_radius < 0.5 {
                                1
                            } else {
                                crate::effects::DROP_SHADOW_KERNEL_TAPS
                            });
                }
                composite_style_pixel(
                    canvas,
                    colored_shadow_pixel(shadow, alpha),
                    scaled_layer.opacity,
                    scaled_layer.clip_to_below,
                    clip,
                    canvas_x as u32 - region.x,
                    canvas_y as u32 - region.y,
                );
            }
        }
    }

    if let Some(intersection) = intersection {
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
                    canvas_x as u32,
                    canvas_y as u32,
                );
            }
        }
    }
    Ok(true)
}

#[derive(Clone, Copy)]
struct ShadowAlphaBounds {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl ShadowAlphaBounds {
    fn pixel_count(self) -> u64 {
        (self.right - self.left) as u64 * (self.bottom - self.top) as u64
    }
}

fn shadow_alpha_bounds(
    geometry: &SamplingGeometry,
    intersection: CanvasIntersection,
    shadow: crate::DropShadow,
) -> Option<ShadowAlphaBounds> {
    let center_left = intersection.left - geometry.origin_x - shadow.offset_x.round() as i64;
    let center_top = intersection.top - geometry.origin_y - shadow.offset_y.round() as i64;
    let center_right = intersection.right - geometry.origin_x - shadow.offset_x.round() as i64;
    let center_bottom = intersection.bottom - geometry.origin_y - shadow.offset_y.round() as i64;
    let (offset_left, offset_top, offset_right, offset_bottom) =
        shadow_kernel_bounds(shadow.blur_radius);
    let left = (center_left + offset_left).max(0);
    let top = (center_top + offset_top).max(0);
    let right = (center_right + offset_right)
        .min(i64::from(geometry.output_width))
        .max(left);
    let bottom = (center_bottom + offset_bottom)
        .min(i64::from(geometry.output_height))
        .max(top);
    (right > left && bottom > top).then_some(ShadowAlphaBounds {
        left,
        top,
        right,
        bottom,
    })
}

fn shadow_kernel_bounds(radius: f32) -> (i64, i64, i64, i64) {
    if radius < 0.5 {
        return (0, 0, 0, 0);
    }
    DROP_SHADOW_KERNEL.into_iter().fold(
        (0, 0, 0, 0),
        |(left, top, right, bottom), (unit_x, unit_y, _)| {
            let x = (unit_x * radius).round() as i64;
            let y = (unit_y * radius).round() as i64;
            (left.min(x), top.min(y), right.max(x), bottom.max(y))
        },
    )
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
        sin: f32,
        cos: f32,
    },
    TextPivot {
        sin: f32,
        cos: f32,
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
                let (sin, cos) = crate::transform_math::rotation_sin_cos(degrees);
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
                        sin,
                        cos,
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
            LayerKind::Raster { .. }
            | LayerKind::Rectangle { .. }
            | LayerKind::Ellipse { .. }
            | LayerKind::Path { .. }
            | LayerKind::Paint { .. }
                if degrees.abs() >= 0.01 =>
            {
                let bounds = centered_rotation_bounds(scaled_width, scaled_height, degrees);
                let (sin, cos) = crate::transform_math::rotation_sin_cos(degrees);
                (
                    bounds.width,
                    bounds.height,
                    (bounds.offset_x, bounds.offset_y),
                    RotationSampling::Center { sin, cos },
                )
            }
            LayerKind::Raster { .. }
            | LayerKind::Rectangle { .. }
            | LayerKind::Ellipse { .. }
            | LayerKind::Path { .. }
            | LayerKind::Paint { .. } => (
                scaled_width,
                scaled_height,
                (0.0, 0.0),
                RotationSampling::None,
            ),
        };
        let path_source_offset = match descriptor {
            SourceDescriptor::Path { scale, .. } => (
                crate::paths::path_source_bounds(base_layer)
                    .expect("path descriptor has path bounds")
                    .origin[0]
                    * scale[0]
                    * scaled_layer.transform.scale_x,
                crate::paths::path_source_bounds(base_layer)
                    .expect("path descriptor has path bounds")
                    .origin[1]
                    * scale[1]
                    * scaled_layer.transform.scale_y,
            ),
            _ => (0.0, 0.0),
        };
        Ok(Self {
            source_width,
            source_height,
            scaled_width,
            scaled_height,
            output_width,
            output_height,
            origin_x: (scaled_layer.transform.x + path_source_offset.0 + offset.0).round() as i64,
            origin_y: (scaled_layer.transform.y + path_source_offset.1 + offset.1).round() as i64,
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

    fn shadow_intersection(
        self,
        region: RenderRegion,
        shadow: crate::DropShadow,
    ) -> Option<CanvasIntersection> {
        let radius = shadow.blur_radius.ceil() as i64;
        let left =
            (self.origin_x + shadow.offset_x.round() as i64 - radius).max(i64::from(region.x));
        let top =
            (self.origin_y + shadow.offset_y.round() as i64 - radius).max(i64::from(region.y));
        let right = (self.origin_x
            + i64::from(self.output_width)
            + shadow.offset_x.round() as i64
            + radius)
            .min(i64::from(region.x + region.width));
        let bottom = (self.origin_y
            + i64::from(self.output_height)
            + shadow.offset_y.round() as i64
            + radius)
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
            RotationSampling::Center { sin, cos } => inverse_center_rotation(
                output_x,
                output_y,
                (self.output_width, self.output_height),
                (self.scaled_width, self.scaled_height),
                (sin, cos),
            ),
            RotationSampling::TextPivot {
                sin,
                cos,
                pivot,
                minimum,
            } => inverse_text_rotation(
                output_x,
                output_y,
                sin,
                cos,
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

fn required_shadow_source_region(
    geometry: &SamplingGeometry,
    intersection: CanvasIntersection,
    shadow: crate::DropShadow,
) -> Option<SourceRegion> {
    if let Some(bounds) = shadow_alpha_bounds(geometry, intersection, shadow)
        && bounds.pixel_count() <= MAX_SOURCE_STAGING_PIXELS
    {
        let tile_intersection = CanvasIntersection {
            left: geometry.origin_x + bounds.left,
            top: geometry.origin_y + bounds.top,
            right: geometry.origin_x + bounds.right,
            bottom: geometry.origin_y + bounds.bottom,
        };
        return required_source_region(geometry, tile_intersection);
    }
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for canvas_y in intersection.top..intersection.bottom {
        for canvas_x in intersection.left..intersection.right {
            let center_x = canvas_x - geometry.origin_x - shadow.offset_x.round() as i64;
            let center_y = canvas_y - geometry.origin_y - shadow.offset_y.round() as i64;
            let _ = drop_shadow_alpha(center_x, center_y, shadow.blur_radius, |x, y| {
                if x >= 0
                    && y >= 0
                    && x < i64::from(geometry.output_width)
                    && y < i64::from(geometry.output_height)
                    && let Some((scaled_x, scaled_y)) = geometry.inverse_sample(x as u32, y as u32)
                {
                    let x = source_sample_bounds(
                        geometry.source_width,
                        geometry.scaled_width,
                        scaled_x,
                    );
                    let y = source_sample_bounds(
                        geometry.source_height,
                        geometry.scaled_height,
                        scaled_y,
                    );
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
                0
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

fn union_source_regions(
    left: Option<SourceRegion>,
    right: Option<SourceRegion>,
) -> Option<SourceRegion> {
    match (left, right) {
        (Some(left), Some(right)) => {
            let x = left.x.min(right.x);
            let y = left.y.min(right.y);
            let far_x = (left.x + left.width).max(right.x + right.width);
            let far_y = (left.y + left.height).max(right.y + right.height);
            Some(SourceRegion {
                x,
                y,
                width: far_x - x,
                height: far_y - y,
            })
        }
        (left, right) => left.or(right),
    }
}

fn sample_output_alpha(
    source: &SampleSource<'_>,
    geometry: &SamplingGeometry,
    layer: &Layer,
    output_x: i64,
    output_y: i64,
) -> u8 {
    if output_x < 0
        || output_y < 0
        || output_x >= i64::from(geometry.output_width)
        || output_y >= i64::from(geometry.output_height)
    {
        return 0;
    }
    let Some((scaled_x, scaled_y)) = geometry.inverse_sample(output_x as u32, output_y as u32)
    else {
        return 0;
    };
    if !crate::render::layer_mask_allows(
        layer,
        output_x as u32,
        output_y as u32,
        geometry.output_width,
        geometry.output_height,
    ) {
        return 0;
    }
    sample_triangle_resize_alpha(
        source,
        (geometry.source_width, geometry.source_height),
        (geometry.scaled_width, geometry.scaled_height),
        (scaled_x, scaled_y),
    )
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
    output: (u32, u32),
    source: (u32, u32),
    direction: (f32, f32),
) -> Option<(u32, u32)> {
    let source_center = ((source.0 as f32 - 1.0) * 0.5, (source.1 as f32 - 1.0) * 0.5);
    let output_center = ((output.0 - 1) as f32 * 0.5, (output.1 - 1) as f32 * 0.5);
    let dx = output_x as f32 - output_center.0;
    let dy = output_y as f32 - output_center.1;
    let source_x = direction.1 * dx + direction.0 * dy + source_center.0;
    let source_y = -direction.0 * dx + direction.1 * dy + source_center.1;
    rounded_source_sample(source_x, source_y, source.0, source.1)
}

fn inverse_text_rotation(
    output_x: u32,
    output_y: u32,
    sin: f32,
    cos: f32,
    pivot: (f32, f32),
    minimum: (f32, f32),
    source: (u32, u32),
) -> Option<(u32, u32)> {
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
