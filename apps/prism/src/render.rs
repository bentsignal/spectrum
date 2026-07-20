use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageEncoder, Rgba, RgbaImage, imageops::FilterType};
use spectrum_imaging::{RenderOptions, render_image};

use crate::{
    Document, Layer, LayerKind, Transform, blend_rgb,
    shapes::{constrained_shape_scale, render_shape},
    text_render::{measure_text, measure_text_geometry, render_text},
};

pub fn save_document(document: &Document, path: &Path) -> Result<()> {
    let extension = path.extension().and_then(|value| value.to_str());
    if !matches!(extension, Some("prism" | "mica")) {
        bail!("Prism projects must use the .prism extension");
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let directory = fs::canonicalize(
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new(".")),
    )?;
    let project_stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("prism");
    let asset_directory = directory.join(format!("{project_stem}-assets"));
    let mut portable = document.clone();
    for layer in &mut portable.layers {
        if let LayerKind::Raster {
            path: source,
            original_path,
        } = &mut layer.kind
        {
            let canonical = fs::canonicalize(&*source)
                .with_context(|| format!("could not read layer source {}", source.display()))?;
            if original_path.is_none() {
                *original_path = Some(canonical.clone());
            }
            if let Ok(relative) = canonical.strip_prefix(&directory) {
                *source = relative.to_owned();
            } else {
                fs::create_dir_all(&asset_directory)?;
                let file_name = canonical
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("image");
                let destination = asset_directory.join(format!("layer-{}-{file_name}", layer.id));
                fs::copy(&canonical, &destination).with_context(|| {
                    format!(
                        "could not copy {} into portable Prism assets",
                        canonical.display()
                    )
                })?;
                *source = destination.strip_prefix(&directory)?.to_owned();
            }
        }
    }
    crate::typography::make_fonts_portable(
        &mut portable.font_assets,
        &directory,
        &asset_directory,
    )?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, serde_json::to_vec_pretty(&portable)?)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    #[cfg(not(target_os = "windows"))]
    fs::rename(&temporary, path)
        .with_context(|| format!("could not replace {}", path.display()))?;
    #[cfg(target_os = "windows")]
    replace_file_windows_safe(&temporary, path)?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn replace_file_windows_safe(temporary: &Path, destination: &Path) -> Result<()> {
    if !destination.exists() {
        fs::rename(temporary, destination)?;
        return Ok(());
    }
    let mut backup = destination.as_os_str().to_owned();
    backup.push(".backup");
    let backup = PathBuf::from(backup);
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    fs::rename(destination, &backup)?;
    match fs::rename(temporary, destination) {
        Ok(()) => {
            fs::remove_file(backup)?;
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup, destination);
            Err(error).with_context(|| format!("could not replace {}", destination.display()))
        }
    }
}

pub fn load_document(path: &Path) -> Result<Document> {
    let bytes = fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    let mut document: Document = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid Prism project {}", path.display()))?;
    document.migrate()?;
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    for layer in &mut document.layers {
        if let LayerKind::Raster { path, .. } = &mut layer.kind
            && path.is_relative()
        {
            *path = directory.join(&*path);
            if let Ok(canonical) = fs::canonicalize(&*path) {
                *path = canonical;
            }
        }
    }
    crate::typography::resolve_portable_fonts(&mut document.font_assets, directory);
    Ok(document)
}

pub fn render_document(document: &Document, max_size: Option<u32>) -> Result<DynamicImage> {
    let longest = document.width.max(document.height) as f32;
    let scale = max_size
        .filter(|size| *size > 0)
        .map_or(1.0, |size| (size as f32 / longest).min(1.0));
    render_document_scaled(document, scale)
}

/// A physical-pixel subregion of a scaled Prism document.
///
/// Interactive clients use regions to keep preview allocation proportional to
/// the visible viewport instead of the full document at the current zoom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Allocation accounting for exact viewport compositing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionRenderStats {
    pub output_pixels: u64,
    pub source_staging_pixels: u64,
    pub source_staging_bytes: u64,
    pub max_source_staging_pixels: u64,
    pub full_source_pixels: u64,
    pub fallback_decode_bytes: u64,
    /// The region path never materializes transformed full-layer surfaces.
    pub transformed_surface_pixels: u64,
}

/// Whether every visible layer can be sampled directly into a viewport region
/// without allocating a transformed full-layer surface.
pub fn document_supports_region_native_zoom(document: &Document) -> bool {
    document.layers.iter().all(|layer| {
        !layer.visible
            || layer.opacity <= 0.0
            || crate::render_region::supports_bounded_source(layer)
            || matches!(
                &layer.kind,
                LayerKind::Rectangle {
                    corner_radius,
                    ..
                } if *corner_radius <= 0.0
            ) && !layer.stroke.enabled
                && layer.adjustments == spectrum_imaging::Adjustments::default()
                && layer.transform.rotation.abs() < 0.01
                && layer.transform.scale_x > 0.0
                && layer.transform.scale_y > 0.0
    })
}

/// Renders a complete document at an explicit canvas-pixel scale. Interactive
/// offscreen clients use this to match export semantics at physical display
/// resolution, including scales above 1 for editable parametric geometry.
pub fn render_document_scaled(document: &Document, scale: f32) -> Result<DynamicImage> {
    let (canvas_width, canvas_height) = scaled_document_dimensions(document, scale)?;
    if canvas_width > crate::MAX_CANVAS_DIMENSION || canvas_height > crate::MAX_CANVAS_DIMENSION {
        bail!("scaled document exceeds Prism's maximum canvas dimension");
    }
    render_document_region_scaled_impl(
        document,
        scale,
        RenderRegion {
            x: 0,
            y: 0,
            width: canvas_width,
            height: canvas_height,
        },
        false,
        &mut RegionRenderStats::default(),
    )
}

/// Renders an exact crop of a document at an explicit scale.
///
/// This shares the export compositor and blend math, but only allocates the
/// requested canvas region. Layer sources are still rasterized at the target
/// scale so text and editable shapes retain high-zoom fidelity.
pub fn render_document_region_scaled(
    document: &Document,
    scale: f32,
    region: RenderRegion,
) -> Result<DynamicImage> {
    render_document_region_scaled_impl(
        document,
        scale,
        region,
        true,
        &mut RegionRenderStats::default(),
    )
}

/// Renders a viewport crop with allocation counters for regression checks.
pub fn render_document_region_scaled_with_stats(
    document: &Document,
    scale: f32,
    region: RenderRegion,
) -> Result<(DynamicImage, RegionRenderStats)> {
    let mut stats = RegionRenderStats::default();
    let image = render_document_region_scaled_impl(document, scale, region, true, &mut stats)?;
    Ok((image, stats))
}

fn render_document_region_scaled_impl(
    document: &Document,
    scale: f32,
    region: RenderRegion,
    bound_fallback_layers: bool,
    stats: &mut RegionRenderStats,
) -> Result<DynamicImage> {
    let (canvas_width, canvas_height) = scaled_document_dimensions(document, scale)?;
    if region.width == 0 || region.height == 0 {
        bail!("document render region must have positive dimensions");
    }
    let right = region
        .x
        .checked_add(region.width)
        .context("document render region overflows horizontally")?;
    let bottom = region
        .y
        .checked_add(region.height)
        .context("document render region overflows vertically")?;
    if right > canvas_width || bottom > canvas_height {
        bail!("document render region exceeds the scaled canvas");
    }
    if region.width > crate::MAX_CANVAS_DIMENSION || region.height > crate::MAX_CANVAS_DIMENSION {
        bail!("document render region exceeds Prism's maximum canvas dimension");
    }
    if bound_fallback_layers && u64::from(region.width) * u64::from(region.height) > 4_096 * 4_096 {
        bail!("document render region exceeds the bounded viewport area");
    }
    stats.output_pixels = u64::from(region.width) * u64::from(region.height);

    let mut canvas = RgbaImage::from_pixel(region.width, region.height, Rgba(document.background));
    let mut previous_coverage: Option<RgbaImage> = None;
    for layer in &document.layers {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        let text_scale = text_raster_scale(layer, scale);
        let shape_scale = if matches!(
            layer.kind,
            LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. }
        ) {
            constrained_shape_scale(
                layer,
                [
                    (layer.transform.scale_x.abs() * scale).max(1.0),
                    (layer.transform.scale_y.abs() * scale).max(1.0),
                ],
                document.width.max(document.height),
            )?
        } else {
            [1.0; 2]
        };
        let mut render_layer = layer.clone();
        if let LayerKind::Text { font_size, .. } = &mut render_layer.kind {
            *font_size *= text_scale;
        }
        let mut scaled_layer = layer.clone();
        scaled_layer.transform.x *= scale;
        scaled_layer.transform.y *= scale;
        scaled_layer.transform.scale_x *= scale / text_scale / shape_scale[0];
        scaled_layer.transform.scale_y *= scale / text_scale / shape_scale[1];
        let mut coverage = RgbaImage::new(region.width, region.height);
        if bound_fallback_layers
            && composite_solid_rectangle_region(
                &mut canvas,
                &mut coverage,
                &render_layer,
                &scaled_layer,
                shape_scale,
                previous_coverage.as_ref(),
                region,
            )
        {
            previous_coverage = Some(coverage);
            continue;
        }
        if bound_fallback_layers
            && crate::render_region::composite_bounded_source_region(
                &mut canvas,
                &mut coverage,
                layer,
                &render_layer,
                &scaled_layer,
                previous_coverage.as_ref(),
                region,
                stats,
            )?
        {
            previous_coverage = Some(coverage);
            continue;
        }
        if bound_fallback_layers {
            ensure_region_fallback_is_bounded(&render_layer, &scaled_layer, shape_scale)?;
        }
        let source = render_layer_preview_scaled(&render_layer, None, shape_scale)?;
        let source = if matches!(render_layer.kind, LayerKind::Text { .. }) {
            let LayerKind::Text {
                text: base_text,
                font_size: base_font_size,
                ..
            } = &layer.kind
            else {
                unreachable!("render layer kind mirrors its source layer");
            };
            let base_geometry = measure_text_geometry(base_text, *base_font_size)?;
            let base_pivot = base_geometry.visual_center();
            let transformed_width = (source.width() as f32 * scaled_layer.transform.scale_x)
                .round()
                .max(1.0);
            let transformed_height = (source.height() as f32 * scaled_layer.transform.scale_y)
                .round()
                .max(1.0);
            let pivot = (
                base_pivot.0 * layer.transform.scale_x * scale * source.width() as f32
                    / transformed_width,
                base_pivot.1 * layer.transform.scale_y * scale * source.height() as f32
                    / transformed_height,
            );
            let (source, offset) =
                crate::text_rotation::transform_text_layer(source, scaled_layer.transform, pivot);
            scaled_layer.transform.x += offset.0;
            scaled_layer.transform.y += offset.1;
            source
        } else {
            transform_layer(source, scaled_layer.transform)
        };
        composite_layer_region(
            &mut canvas,
            &mut coverage,
            &source,
            &scaled_layer,
            previous_coverage.as_ref(),
            region,
        );
        previous_coverage = Some(coverage);
    }
    Ok(DynamicImage::ImageRgba8(canvas))
}

fn ensure_region_fallback_is_bounded(
    render_layer: &Layer,
    scaled_layer: &Layer,
    shape_scale: [f32; 2],
) -> Result<()> {
    const MAX_FALLBACK_PIXELS: u64 = 4_096 * 4_096;
    let (base_width, base_height) = match &render_layer.kind {
        LayerKind::Raster { path, .. } => image::image_dimensions(path)
            .with_context(|| format!("could not inspect layer source {}", path.display()))?,
        LayerKind::Text {
            text, font_size, ..
        } => measure_text(text, *font_size)?,
        LayerKind::Rectangle { width, height, .. } | LayerKind::Ellipse { width, height, .. } => (
            (*width as f32 * shape_scale[0]).round().max(1.0) as u32,
            (*height as f32 * shape_scale[1]).round().max(1.0) as u32,
        ),
    };
    let scaled_width = (base_width as f32 * scaled_layer.transform.scale_x)
        .abs()
        .round()
        .max(1.0) as u32;
    let scaled_height = (base_height as f32 * scaled_layer.transform.scale_y)
        .abs()
        .round()
        .max(1.0) as u32;
    let (transformed_width, transformed_height) = if scaled_layer.transform.rotation.abs() < 0.01 {
        (scaled_width, scaled_height)
    } else {
        let radians = scaled_layer.transform.rotation.to_radians();
        let (sin, cos) = radians.sin_cos();
        (
            (scaled_width as f32 * cos.abs() + scaled_height as f32 * sin.abs())
                .ceil()
                .max(1.0) as u32,
            (scaled_width as f32 * sin.abs() + scaled_height as f32 * cos.abs())
                .ceil()
                .max(1.0) as u32,
        )
    };
    let base_pixels = u64::from(base_width) * u64::from(base_height);
    let scaled_pixels = u64::from(scaled_width) * u64::from(scaled_height);
    let transformed_pixels = u64::from(transformed_width) * u64::from(transformed_height);
    if base_width > crate::MAX_CANVAS_DIMENSION
        || base_height > crate::MAX_CANVAS_DIMENSION
        || transformed_width > crate::MAX_CANVAS_DIMENSION
        || transformed_height > crate::MAX_CANVAS_DIMENSION
        || base_pixels > MAX_FALLBACK_PIXELS
        || scaled_pixels > MAX_FALLBACK_PIXELS
        || transformed_pixels > MAX_FALLBACK_PIXELS
    {
        bail!(
            "layer {} exceeds the bounded viewport fallback; lower zoom or simplify the layer",
            render_layer.id
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn composite_solid_rectangle_region(
    canvas: &mut RgbaImage,
    coverage: &mut RgbaImage,
    render_layer: &Layer,
    scaled_layer: &Layer,
    shape_scale: [f32; 2],
    clip: Option<&RgbaImage>,
    region: RenderRegion,
) -> bool {
    let LayerKind::Rectangle {
        width,
        height,
        color,
        corner_radius,
    } = &render_layer.kind
    else {
        return false;
    };
    if *corner_radius > 0.0
        || render_layer.stroke.enabled
        || render_layer.adjustments != spectrum_imaging::Adjustments::default()
        || scaled_layer.transform.rotation.abs() >= 0.01
        || scaled_layer.transform.scale_x <= 0.0
        || scaled_layer.transform.scale_y <= 0.0
    {
        return false;
    }

    let base_width = (*width as f32 * shape_scale[0]).round().max(1.0) as u32;
    let base_height = (*height as f32 * shape_scale[1]).round().max(1.0) as u32;
    let source_width = (base_width as f32 * scaled_layer.transform.scale_x)
        .round()
        .max(1.0) as u32;
    let source_height = (base_height as f32 * scaled_layer.transform.scale_y)
        .round()
        .max(1.0) as u32;
    let origin_x = scaled_layer.transform.x.round() as i64;
    let origin_y = scaled_layer.transform.y.round() as i64;
    let left = origin_x.max(i64::from(region.x));
    let top = origin_y.max(i64::from(region.y));
    let right = (origin_x + i64::from(source_width)).min(i64::from(region.x + region.width));
    let bottom = (origin_y + i64::from(source_height)).min(i64::from(region.y + region.height));
    if right <= left || bottom <= top {
        return true;
    }

    for canvas_y in top..bottom {
        for canvas_x in left..right {
            let source_x = (canvas_x - origin_x) as u32;
            let source_y = (canvas_y - origin_y) as u32;
            composite_pixel(
                canvas,
                coverage,
                *color,
                source_x,
                source_y,
                source_width,
                source_height,
                scaled_layer,
                clip,
                canvas_x as u32 - region.x,
                canvas_y as u32 - region.y,
            );
        }
    }
    true
}

fn scaled_document_dimensions(document: &Document, scale: f32) -> Result<(u32, u32)> {
    if !scale.is_finite() || scale <= 0.0 {
        bail!("document render scale must be a positive finite number");
    }
    let width = (document.width as f64 * f64::from(scale)).round().max(1.0);
    let height = (document.height as f64 * f64::from(scale)).round().max(1.0);
    if width > u32::MAX as f64 || height > u32::MAX as f64 {
        bail!("scaled document dimensions overflow");
    }
    Ok((width as u32, height as u32))
}

pub fn render_document_thumbnail(document: &Document, max_size: u32) -> Result<DynamicImage> {
    render_document(document, Some(max_size))
}

/// Renders one layer's source pixels without its canvas transform, opacity, or blend mode.
/// Interactive clients can cache this result and apply transforms on the GPU.
pub fn render_layer_preview(layer: &Layer, max_size: Option<u32>) -> Result<DynamicImage> {
    render_layer_preview_scaled(layer, max_size, [1.0; 2])
}

pub fn render_layer_preview_scaled(
    layer: &Layer,
    max_size: Option<u32>,
    shape_scale: [f32; 2],
) -> Result<DynamicImage> {
    let image = render_layer_base_scaled(layer, max_size, shape_scale)?;
    Ok(render_image(
        image,
        layer.adjustments.clone(),
        RenderOptions::default(),
    ))
}

/// Decodes or rasterizes a layer without development adjustments. Keeping this
/// result cached avoids repeatedly decoding large linked images during sliders.
pub fn render_layer_base(layer: &Layer, max_size: Option<u32>) -> Result<DynamicImage> {
    render_layer_base_scaled(layer, max_size, [1.0; 2])
}

pub fn render_layer_base_scaled(
    layer: &Layer,
    max_size: Option<u32>,
    shape_scale: [f32; 2],
) -> Result<DynamicImage> {
    let mut image = match &layer.kind {
        LayerKind::Raster { path, .. } => image::ImageReader::open(path)
            .with_context(|| format!("could not open {}", path.display()))?
            .with_guessed_format()?
            .decode()
            .with_context(|| format!("could not decode {}", path.display()))?,
        LayerKind::Text {
            text,
            font_size,
            color,
            ..
        } => DynamicImage::ImageRgba8(render_text(text, *font_size, *color)?),
        LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } => {
            DynamicImage::ImageRgba8(render_shape(layer, shape_scale)?)
        }
    };
    if let Some(max_size) =
        max_size.filter(|size| *size > 0 && (image.width() > *size || image.height() > *size))
    {
        image = image.resize(max_size, max_size, FilterType::Triangle);
    }
    Ok(image)
}

/// Applies development adjustments to a uniform color in constant time.
/// This keeps vector-style shape sliders responsive without rasterizing the shape.
pub fn render_solid_color(color: [u8; 4], adjustments: &spectrum_imaging::Adjustments) -> [u8; 4] {
    let image = RgbaImage::from_pixel(1, 1, Rgba(color));
    render_image(
        DynamicImage::ImageRgba8(image),
        adjustments.clone(),
        RenderOptions::default(),
    )
    .to_rgba8()
    .get_pixel(0, 0)
    .0
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

fn transform_layer(image: DynamicImage, transform: Transform) -> RgbaImage {
    let width = (image.width() as f32 * transform.scale_x).round().max(1.0) as u32;
    let height = (image.height() as f32 * transform.scale_y).round().max(1.0) as u32;
    let scaled = image
        .resize_exact(width, height, FilterType::Triangle)
        .to_rgba8();
    if transform.rotation.abs() < 0.01 {
        return scaled;
    }
    rotate_rgba(&scaled, transform.rotation)
}

fn rotate_rgba(source: &RgbaImage, degrees: f32) -> RgbaImage {
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let width = source.width() as f32;
    let height = source.height() as f32;
    let output_width = (width * cos.abs() + height * sin.abs()).ceil().max(1.0) as u32;
    let output_height = (width * sin.abs() + height * cos.abs()).ceil().max(1.0) as u32;
    let source_center = ((width - 1.0) * 0.5, (height - 1.0) * 0.5);
    let output_center = (
        (output_width - 1) as f32 * 0.5,
        (output_height - 1) as f32 * 0.5,
    );
    let mut output = RgbaImage::new(output_width, output_height);
    for y in 0..output_height {
        for x in 0..output_width {
            let dx = x as f32 - output_center.0;
            let dy = y as f32 - output_center.1;
            let source_x = cos * dx + sin * dy + source_center.0;
            let source_y = -sin * dx + cos * dy + source_center.1;
            if source_x >= 0.0 && source_y >= 0.0 && source_x < width && source_y < height {
                let sample_x = source_x.round().clamp(0.0, width - 1.0) as u32;
                let sample_y = source_y.round().clamp(0.0, height - 1.0) as u32;
                output.put_pixel(x, y, *source.get_pixel(sample_x, sample_y));
            }
        }
    }
    output
}

fn composite_layer_region(
    canvas: &mut RgbaImage,
    coverage: &mut RgbaImage,
    source: &RgbaImage,
    layer: &Layer,
    clip: Option<&RgbaImage>,
    region: RenderRegion,
) {
    let origin_x = layer.transform.x.round() as i64;
    let origin_y = layer.transform.y.round() as i64;
    let source_left = (i64::from(region.x) - origin_x).clamp(0, i64::from(source.width())) as u32;
    let source_top = (i64::from(region.y) - origin_y).clamp(0, i64::from(source.height())) as u32;
    let source_right =
        (i64::from(region.x + region.width) - origin_x).clamp(0, i64::from(source.width())) as u32;
    let source_bottom = (i64::from(region.y + region.height) - origin_y)
        .clamp(0, i64::from(source.height())) as u32;
    for source_y in source_top..source_bottom {
        for source_x in source_left..source_right {
            let source_pixel = source.get_pixel(source_x, source_y);
            let canvas_x = origin_x + i64::from(source_x);
            let canvas_y = origin_y + i64::from(source_y);
            let x = canvas_x as u32 - region.x;
            let y = canvas_y as u32 - region.y;
            composite_pixel(
                canvas,
                coverage,
                source_pixel.0,
                source_x,
                source_y,
                source.width(),
                source.height(),
                layer,
                clip,
                x,
                y,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn composite_pixel(
    canvas: &mut RgbaImage,
    coverage: &mut RgbaImage,
    source_pixel: [u8; 4],
    source_x: u32,
    source_y: u32,
    source_width: u32,
    source_height: u32,
    layer: &Layer,
    clip: Option<&RgbaImage>,
    x: u32,
    y: u32,
) {
    let normalized_x = source_x as f32 / source_width.max(1) as f32;
    let normalized_y = source_y as f32 / source_height.max(1) as f32;
    let in_mask = normalized_x >= layer.mask.x
        && normalized_x <= layer.mask.x + layer.mask.width
        && normalized_y >= layer.mask.y
        && normalized_y <= layer.mask.y + layer.mask.height;
    let mask_alpha = if !layer.mask.enabled || in_mask != layer.mask.invert {
        1.0
    } else {
        0.0
    };
    let clip_alpha = if layer.clip_to_below {
        clip.map_or(0.0, |image| image.get_pixel(x, y)[3] as f32 / 255.0)
    } else {
        1.0
    };
    let alpha = source_pixel[3] as f32 / 255.0 * layer.opacity * mask_alpha * clip_alpha;
    if alpha <= 0.0 {
        return;
    }
    let destination = *canvas.get_pixel(x, y);
    let blended = blend_rgb(source_pixel, destination.0, layer.blend_mode);
    let destination_alpha = destination[3] as f32 / 255.0;
    let output_alpha = alpha + destination_alpha * (1.0 - alpha);
    let mut output = [0; 4];
    for channel in 0..3 {
        let value = if output_alpha > 0.0 {
            (source_pixel[channel] as f32 * alpha * (1.0 - destination_alpha)
                + blended[channel] as f32 * alpha * destination_alpha
                + destination[channel] as f32 * destination_alpha * (1.0 - alpha))
                / output_alpha
        } else {
            0.0
        };
        output[channel] = value.round().clamp(0.0, 255.0) as u8;
    }
    output[3] = (output_alpha * 255.0).round() as u8;
    canvas.put_pixel(x, y, Rgba(output));
    coverage.put_pixel(x, y, Rgba([255, 255, 255, (alpha * 255.0) as u8]));
}

pub fn export_document(document: &Document, path: &Path, quality: u8) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "jpg" | "jpeg" | "png") {
        bail!("export path must end in .png, .jpg, or .jpeg");
    }
    let destination = if path.exists() {
        fs::canonicalize(path)?
    } else {
        let parent = fs::canonicalize(
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new(".")),
        )?;
        parent.join(path.file_name().context("export path needs a file name")?)
    };
    for layer in &document.layers {
        if let LayerKind::Raster {
            path: source,
            original_path,
        } = &layer.kind
        {
            let overwrites_source = fs::canonicalize(source).ok().as_ref() == Some(&destination);
            let overwrites_original = original_path.as_ref().is_some_and(|original| {
                fs::canonicalize(original).ok().as_ref() == Some(&destination)
            });
            if overwrites_source || overwrites_original {
                bail!(
                    "refusing to overwrite raster source {}; choose a new export path",
                    if overwrites_original {
                        original_path.as_ref().unwrap_or(source)
                    } else {
                        source
                    }
                    .display()
                );
            }
        }
    }
    let image = render_document(document, None)?;
    let file =
        fs::File::create(path).with_context(|| format!("could not create {}", path.display()))?;
    let writer = BufWriter::new(file);
    match extension.as_str() {
        "jpg" | "jpeg" => {
            let rgb = image.to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(writer, quality.clamp(1, 100))
                .write_image(
                    &rgb,
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )?;
        }
        "png" => {
            let rgba = image.to_rgba8();
            image::codecs::png::PngEncoder::new(writer).write_image(
                &rgba,
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )?;
        }
        _ => unreachable!("extension was validated before rendering"),
    }
    Ok(())
}

#[cfg(test)]
mod text_tests {
    use super::*;
    use fontdue::Font;

    #[test]
    fn glyph_layout_does_not_discard_descender_pixels() {
        let font = Font::from_bytes(
            epaint_default_fonts::UBUNTU_LIGHT,
            fontdue::FontSettings::default(),
        )
        .unwrap();
        let (_, glyph) = font.rasterize('g', 72.0);
        let rendered = render_text("g", 72.0, [255, 255, 255, 255]).unwrap();
        let source_alpha: u64 = glyph.into_iter().map(u64::from).sum();
        let rendered_alpha: u64 = rendered.pixels().map(|pixel| u64::from(pixel[3])).sum();
        assert_eq!(rendered_alpha, source_alpha);
    }
}
