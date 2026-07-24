use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};

use crate::{Document, Layer, LayerKind, MAX_CANVAS_DIMENSION};

const MAX_INTERACTIVE_SHAPE_RASTER: u32 = 8_192;
const MAX_RASTERIZE_SCALE: f32 = 64.0;
static NEXT_GENERATED_ASSET: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq)]
pub struct RasterizedShapeAsset {
    pub path: PathBuf,
    pub scale: f32,
}

pub fn rasterize_shape_asset(
    document: &Document,
    id: u64,
    requested_scale: f32,
) -> Result<RasterizedShapeAsset> {
    if !requested_scale.is_finite() || requested_scale <= 0.0 {
        bail!("rasterization scale must be a positive finite number");
    }
    let layer = document.layer(id)?;
    let constrained = constrained_shape_scale(
        layer,
        [requested_scale.min(MAX_RASTERIZE_SCALE); 2],
        MAX_CANVAS_DIMENSION,
    )?;
    let scale = constrained[0].min(constrained[1]);
    let mut image = render_shape(layer, [scale; 2])?;
    let (width, height) = image.dimensions();
    crate::paths::apply_vector_mask_to_image(
        &mut image,
        layer.vector_mask.as_ref(),
        width,
        height,
        0,
        0,
    )?;
    let directory = std::env::temp_dir().join("spectrum-prism-generated");
    fs::create_dir_all(&directory)
        .with_context(|| format!("could not create {}", directory.display()))?;
    let sequence = NEXT_GENERATED_ASSET.fetch_add(1, Ordering::Relaxed);
    let path = directory.join(format!("shape-{}-{sequence}.png", std::process::id()));
    image
        .save(&path)
        .with_context(|| format!("could not write rasterized shape {}", path.display()))?;
    Ok(RasterizedShapeAsset { path, scale })
}

pub fn recommended_rasterization_scale(layer: &Layer) -> Result<f32> {
    let desired = layer
        .transform
        .scale_x
        .abs()
        .max(layer.transform.scale_y.abs())
        .max(1.0)
        .ceil();
    let constrained = constrained_shape_scale(layer, [desired; 2], MAX_CANVAS_DIMENSION)?;
    Ok(constrained[0].min(constrained[1]))
}

pub fn interactive_shape_scale(layer: &Layer, zoom: f32) -> Result<[u32; 2]> {
    let desired = [
        quantized_scale(layer.transform.scale_x.abs() * zoom.max(0.1)),
        quantized_scale(layer.transform.scale_y.abs() * zoom.max(0.1)),
    ];
    let constrained = constrained_shape_scale(
        layer,
        [desired[0] as f32, desired[1] as f32],
        MAX_INTERACTIVE_SHAPE_RASTER,
    )?;
    Ok([
        constrained[0].floor().max(1.0) as u32,
        constrained[1].floor().max(1.0) as u32,
    ])
}

/// Whether an interactive path preview must use the viewport-bounded compositor.
///
/// The cached per-layer path texture is intentionally a whole-source raster. At
/// close zooms that raster can exceed the path renderer's bounded working area,
/// even though the visible document region remains small. Callers use this
/// predicate to keep the lower-resolution cached texture as a fallback while
/// the exact tiled region compositor renders the visible pixels.
pub fn path_preview_requires_region(layer: &Layer, zoom: f32) -> Result<bool> {
    if !matches!(layer.kind, LayerKind::Path { .. }) {
        return Ok(false);
    }
    let scale = interactive_shape_scale(layer, zoom)?;
    let bounds =
        crate::paths::path_source_bounds(layer).context("path layer has no source bounds")?;
    let (width, height) = bounds.raster_dimensions([scale[0] as f32, scale[1] as f32])?;
    Ok(u64::from(width) * u64::from(height) > crate::paths::MAX_PATH_RASTER_PIXELS)
}

pub fn shape_dimensions(layer: &Layer) -> Option<(u32, u32)> {
    match &layer.kind {
        LayerKind::Rectangle { width, height, .. } | LayerKind::Ellipse { width, height, .. } => {
            Some((*width, *height))
        }
        LayerKind::Path { .. } => crate::paths::path_source_bounds(layer)
            .and_then(|bounds| bounds.raster_dimensions([1.0; 2]).ok()),
        _ => None,
    }
}

pub(crate) fn constrained_shape_scale(
    layer: &Layer,
    desired: [f32; 2],
    max_dimension: u32,
) -> Result<[f32; 2]> {
    let (width, height) = shape_dimensions(layer).context("layer is not a parametric shape")?;
    let limit_x = max_dimension as f32 / width.max(1) as f32;
    let limit_y = max_dimension as f32 / height.max(1) as f32;
    Ok([
        desired[0].clamp(1.0, limit_x.max(1.0)),
        desired[1].clamp(1.0, limit_y.max(1.0)),
    ])
}

pub(crate) fn render_shape(layer: &Layer, scale: [f32; 2]) -> Result<RgbaImage> {
    if matches!(layer.kind, LayerKind::Path { .. }) {
        return crate::paths::render_path(layer, scale);
    }
    let sampler = ShapeSampler::new(layer, scale)?;
    let (width, height) = sampler.dimensions();
    let mut output = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            output.put_pixel(x, y, sampler.pixel(x, y));
        }
    }
    Ok(output)
}

/// Samples the exact parametric source raster without allocating that raster.
///
/// Region compositing uses this at high zoom so its memory use follows the
/// visible viewport rather than the full scaled shape.
#[derive(Clone, Copy)]
pub(crate) struct ShapeSampler<'a> {
    layer: &'a Layer,
    scale: [f32; 2],
    dimensions: (u32, u32),
    fill_direction: Option<(f32, f32)>,
}

impl<'a> ShapeSampler<'a> {
    pub(crate) fn new(layer: &'a Layer, scale: [f32; 2]) -> Result<Self> {
        if scale
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            bail!("shape render scale must contain positive finite numbers");
        }
        let (width, height) = shape_dimensions(layer).context("layer is not a parametric shape")?;
        let dimensions = (
            scaled_dimension(width, scale[0]),
            scaled_dimension(height, scale[1]),
        );
        if dimensions.0 > MAX_CANVAS_DIMENSION || dimensions.1 > MAX_CANVAS_DIMENSION {
            bail!("shape render exceeds Prism's maximum raster dimension");
        }
        Ok(Self {
            layer,
            scale,
            dimensions,
            fill_direction: layer.shape_fill.as_ref().map(|fill| fill.direction()),
        })
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        self.dimensions
    }

    /// Returns the source pixel when every coordinate has the same value.
    ///
    /// Keeping this property on the sampler lets every compositor consumer
    /// avoid redundant procedural sampling without duplicating shape rules.
    pub(crate) fn uniform_pixel(&self) -> Option<Rgba<u8>> {
        if self.layer.pixel_mask.is_some() {
            return None;
        }
        match self.layer.kind {
            LayerKind::Rectangle {
                color,
                corner_radius,
                ..
            } if corner_radius <= 0.0 && !self.layer.stroke.enabled => match &self.layer.shape_fill
            {
                Some(fill) => fill.uniform_color().map(Rgba),
                None => Some(Rgba(color)),
            },
            _ => None,
        }
    }

    pub(crate) fn pixel(&self, x: u32, y: u32) -> Rgba<u8> {
        let mut color = match self.layer.kind {
            LayerKind::Rectangle {
                width,
                height,
                color,
                corner_radius,
            } => {
                if corner_radius <= 0.0 && !self.layer.stroke.enabled {
                    shape_fill_color(
                        self.layer,
                        color,
                        (x as f32 + 0.5) / self.scale[0],
                        (y as f32 + 0.5) / self.scale[1],
                        width,
                        height,
                        self.fill_direction,
                    )
                } else {
                    sample_shape_pixel(x, y, self.scale, |x, y| {
                        if !rounded_rect_contains(x, y, width, height, corner_radius, 0.0) {
                            return None;
                        }
                        let stroke_pixel = self.layer.stroke.enabled
                            && !rounded_rect_contains(
                                x,
                                y,
                                width,
                                height,
                                corner_radius,
                                self.layer.stroke.width.min(width.min(height) as f32 * 0.5),
                            );
                        Some(if stroke_pixel {
                            self.layer.stroke.color
                        } else {
                            shape_fill_color(
                                self.layer,
                                color,
                                x,
                                y,
                                width,
                                height,
                                self.fill_direction,
                            )
                        })
                    })
                }
            }
            LayerKind::Ellipse {
                width,
                height,
                color,
            } => {
                let center_x = width as f32 * 0.5;
                let center_y = height as f32 * 0.5;
                let radius_x = center_x.max(0.5);
                let radius_y = center_y.max(0.5);
                let inner_x = (radius_x - self.layer.stroke.width).max(0.0);
                let inner_y = (radius_y - self.layer.stroke.width).max(0.0);
                sample_shape_pixel(x, y, self.scale, |x, y| {
                    let dx = x - center_x;
                    let dy = y - center_y;
                    let outer = (dx / radius_x).powi(2) + (dy / radius_y).powi(2) <= 1.0;
                    if !outer {
                        return None;
                    }
                    let inner = inner_x > 0.0
                        && inner_y > 0.0
                        && (dx / inner_x).powi(2) + (dy / inner_y).powi(2) <= 1.0;
                    Some(if self.layer.stroke.enabled && !inner {
                        self.layer.stroke.color
                    } else {
                        shape_fill_color(
                            self.layer,
                            color,
                            x,
                            y,
                            width,
                            height,
                            self.fill_direction,
                        )
                    })
                })
            }
            _ => unreachable!("ShapeSampler validates the layer kind"),
        };
        color[3] = multiply_alpha(color[3], self.pixel_mask_alpha(x, y));
        Rgba(color)
    }

    pub(crate) fn alpha(&self, x: u32, y: u32) -> u8 {
        let alpha = match self.layer.kind {
            LayerKind::Rectangle {
                width,
                height,
                color,
                corner_radius,
            } => {
                if corner_radius <= 0.0 && !self.layer.stroke.enabled {
                    shape_fill_alpha(
                        self.layer,
                        color[3],
                        (x as f32 + 0.5) / self.scale[0],
                        (y as f32 + 0.5) / self.scale[1],
                        width,
                        height,
                        self.fill_direction,
                    )
                } else {
                    sample_shape_alpha(x, y, self.scale, |x, y| {
                        if !rounded_rect_contains(x, y, width, height, corner_radius, 0.0) {
                            return None;
                        }
                        let stroke_pixel = self.layer.stroke.enabled
                            && !rounded_rect_contains(
                                x,
                                y,
                                width,
                                height,
                                corner_radius,
                                self.layer.stroke.width.min(width.min(height) as f32 * 0.5),
                            );
                        Some(if stroke_pixel {
                            self.layer.stroke.color[3]
                        } else {
                            shape_fill_alpha(
                                self.layer,
                                color[3],
                                x,
                                y,
                                width,
                                height,
                                self.fill_direction,
                            )
                        })
                    })
                }
            }
            LayerKind::Ellipse {
                width,
                height,
                color,
            } => {
                let center_x = width as f32 * 0.5;
                let center_y = height as f32 * 0.5;
                let radius_x = center_x.max(0.5);
                let radius_y = center_y.max(0.5);
                let inner_x = (radius_x - self.layer.stroke.width).max(0.0);
                let inner_y = (radius_y - self.layer.stroke.width).max(0.0);
                sample_shape_alpha(x, y, self.scale, |x, y| {
                    let dx = x - center_x;
                    let dy = y - center_y;
                    let outer = (dx / radius_x).powi(2) + (dy / radius_y).powi(2) <= 1.0;
                    if !outer {
                        return None;
                    }
                    let inner = inner_x > 0.0
                        && inner_y > 0.0
                        && (dx / inner_x).powi(2) + (dy / inner_y).powi(2) <= 1.0;
                    Some(if self.layer.stroke.enabled && !inner {
                        self.layer.stroke.color[3]
                    } else {
                        shape_fill_alpha(
                            self.layer,
                            color[3],
                            x,
                            y,
                            width,
                            height,
                            self.fill_direction,
                        )
                    })
                })
            }
            _ => unreachable!("ShapeSampler validates the layer kind"),
        };
        multiply_alpha(alpha, self.pixel_mask_alpha(x, y))
    }

    fn pixel_mask_alpha(&self, x: u32, y: u32) -> u8 {
        let Some(mask) = &self.layer.pixel_mask else {
            return 255;
        };
        let source_x = ((x as f32 + 0.5) / self.scale[0])
            .floor()
            .clamp(0.0, mask.width.saturating_sub(1) as f32) as u32;
        let source_y = ((y as f32 + 0.5) / self.scale[1])
            .floor()
            .clamp(0.0, mask.height.saturating_sub(1) as f32) as u32;
        mask.alpha[(u64::from(source_y) * u64::from(mask.width) + u64::from(source_x)) as usize]
    }
}

fn multiply_alpha(left: u8, right: u8) -> u8 {
    ((u16::from(left) * u16::from(right) + 127) / 255) as u8
}

fn shape_fill_color(
    layer: &Layer,
    fallback: [u8; 4],
    x: f32,
    y: f32,
    width: u32,
    height: u32,
    direction: Option<(f32, f32)>,
) -> [u8; 4] {
    layer
        .shape_fill
        .as_ref()
        .map(|fill| fill.sample(x, y, width, height, direction.unwrap_or((1.0, 0.0))))
        .unwrap_or(fallback)
}

fn shape_fill_alpha(
    layer: &Layer,
    fallback: u8,
    x: f32,
    y: f32,
    width: u32,
    height: u32,
    direction: Option<(f32, f32)>,
) -> u8 {
    layer
        .shape_fill
        .as_ref()
        .map(|fill| fill.sample_alpha(x, y, width, height, direction.unwrap_or((1.0, 0.0))))
        .unwrap_or(fallback)
}

fn sample_shape_pixel(
    x: u32,
    y: u32,
    scale: [f32; 2],
    mut sample: impl FnMut(f32, f32) -> Option<[u8; 4]>,
) -> [u8; 4] {
    const OFFSETS: [(f32, f32); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    let mut alpha = 0_u32;
    let mut premultiplied = [0_u32; 3];
    for (offset_x, offset_y) in OFFSETS {
        let Some(color) = sample(
            (x as f32 + offset_x) / scale[0],
            (y as f32 + offset_y) / scale[1],
        ) else {
            continue;
        };
        alpha += u32::from(color[3]);
        for channel in 0..3 {
            premultiplied[channel] += u32::from(color[channel]) * u32::from(color[3]);
        }
    }
    if alpha == 0 {
        return [0; 4];
    }
    let mut color = [0_u8; 4];
    for channel in 0..3 {
        color[channel] = (premultiplied[channel] / alpha) as u8;
    }
    color[3] = (alpha / OFFSETS.len() as u32) as u8;
    color
}

fn sample_shape_alpha(
    x: u32,
    y: u32,
    scale: [f32; 2],
    mut sample: impl FnMut(f32, f32) -> Option<u8>,
) -> u8 {
    const OFFSETS: [(f32, f32); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    let alpha = OFFSETS
        .into_iter()
        .filter_map(|(offset_x, offset_y)| {
            sample(
                (x as f32 + offset_x) / scale[0],
                (y as f32 + offset_y) / scale[1],
            )
        })
        .map(u32::from)
        .sum::<u32>();
    (alpha / OFFSETS.len() as u32) as u8
}

fn quantized_scale(target: f32) -> u32 {
    (target.max(1.0).ceil() as u32)
        .next_power_of_two()
        .min(MAX_RASTERIZE_SCALE as u32)
}

fn rounded_rect_contains(x: f32, y: f32, width: u32, height: u32, radius: f32, inset: f32) -> bool {
    let left = inset;
    let top = inset;
    let right = width as f32 - inset;
    let bottom = height as f32 - inset;
    if right <= left || bottom <= top || x < left || x > right || y < top || y > bottom {
        return false;
    }
    let radius = (radius - inset)
        .max(0.0)
        .min((right - left).min(bottom - top) * 0.5);
    if radius == 0.0 {
        return true;
    }
    let nearest_x = x.clamp(left + radius, right - radius);
    let nearest_y = y.clamp(top + radius, bottom - radius);
    let dx = x - nearest_x;
    let dy = y - nearest_y;
    dx * dx + dy * dy <= radius * radius
}

fn scaled_dimension(value: u32, scale: f32) -> u32 {
    (value as f32 * scale).round().max(1.0) as u32
}
