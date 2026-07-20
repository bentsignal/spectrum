use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};

use crate::{Document, Layer, LayerKind, MAX_CANVAS_DIMENSION, ShapeStroke};

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
    let image = render_shape(layer, [scale; 2])?;
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

pub fn shape_dimensions(layer: &Layer) -> Option<(u32, u32)> {
    match layer.kind {
        LayerKind::Rectangle { width, height, .. } | LayerKind::Ellipse { width, height, .. } => {
            Some((width, height))
        }
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
    if scale
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        bail!("shape render scale must contain positive finite numbers");
    }
    let (width, height) = shape_dimensions(layer).context("layer is not a parametric shape")?;
    if scaled_dimension(width, scale[0]) > MAX_CANVAS_DIMENSION
        || scaled_dimension(height, scale[1]) > MAX_CANVAS_DIMENSION
    {
        bail!("shape render exceeds Prism's maximum raster dimension");
    }
    match layer.kind {
        LayerKind::Rectangle {
            width,
            height,
            color,
            corner_radius,
        } => Ok(render_rectangle(
            width,
            height,
            color,
            corner_radius,
            layer.stroke,
            scale,
        )),
        LayerKind::Ellipse {
            width,
            height,
            color,
        } => Ok(render_ellipse(width, height, color, layer.stroke, scale)),
        _ => bail!("layer {} is not a parametric shape", layer.id),
    }
}

fn quantized_scale(target: f32) -> u32 {
    (target.max(1.0).ceil() as u32)
        .next_power_of_two()
        .min(MAX_RASTERIZE_SCALE as u32)
}

fn render_rectangle(
    width: u32,
    height: u32,
    color: [u8; 4],
    radius: f32,
    stroke: ShapeStroke,
    scale: [f32; 2],
) -> RgbaImage {
    let output_width = scaled_dimension(width, scale[0]);
    let output_height = scaled_dimension(height, scale[1]);
    if radius <= 0.0 && !stroke.enabled {
        return RgbaImage::from_pixel(output_width, output_height, Rgba(color));
    }
    sample_shape(output_width, output_height, scale, |x, y| {
        if !rounded_rect_contains(x, y, width, height, radius, 0.0) {
            return None;
        }
        let stroke_pixel = stroke.enabled
            && !rounded_rect_contains(
                x,
                y,
                width,
                height,
                radius,
                stroke.width.min(width.min(height) as f32 * 0.5),
            );
        Some(if stroke_pixel { stroke.color } else { color })
    })
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

fn render_ellipse(
    width: u32,
    height: u32,
    color: [u8; 4],
    stroke: ShapeStroke,
    scale: [f32; 2],
) -> RgbaImage {
    let output_width = scaled_dimension(width, scale[0]);
    let output_height = scaled_dimension(height, scale[1]);
    let center_x = width as f32 * 0.5;
    let center_y = height as f32 * 0.5;
    let radius_x = center_x.max(0.5);
    let radius_y = center_y.max(0.5);
    let inner_x = (radius_x - stroke.width).max(0.0);
    let inner_y = (radius_y - stroke.width).max(0.0);
    sample_shape(output_width, output_height, scale, |x, y| {
        let dx = x - center_x;
        let dy = y - center_y;
        let outer = (dx / radius_x).powi(2) + (dy / radius_y).powi(2) <= 1.0;
        if !outer {
            return None;
        }
        let inner = inner_x > 0.0
            && inner_y > 0.0
            && (dx / inner_x).powi(2) + (dy / inner_y).powi(2) <= 1.0;
        Some(if stroke.enabled && !inner {
            stroke.color
        } else {
            color
        })
    })
}

fn scaled_dimension(value: u32, scale: f32) -> u32 {
    (value as f32 * scale).round().max(1.0) as u32
}

fn sample_shape(
    width: u32,
    height: u32,
    scale: [f32; 2],
    mut sample: impl FnMut(f32, f32) -> Option<[u8; 4]>,
) -> RgbaImage {
    const OFFSETS: [(f32, f32); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    let mut output = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
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
                continue;
            }
            let mut color = [0_u8; 4];
            for channel in 0..3 {
                color[channel] = (premultiplied[channel] / alpha) as u8;
            }
            color[3] = (alpha / OFFSETS.len() as u32) as u8;
            output.put_pixel(x, y, Rgba(color));
        }
    }
    output
}
