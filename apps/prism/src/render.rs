use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use fontdue::Font;
use image::{DynamicImage, ImageEncoder, Rgba, RgbaImage, imageops::FilterType};
use spectrum_imaging::{RenderOptions, render_image};

use crate::{
    Document, Layer, LayerKind, Transform, blend_rgb,
    shapes::{constrained_shape_scale, render_shape},
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
    Ok(document)
}

pub fn render_document(document: &Document, max_size: Option<u32>) -> Result<DynamicImage> {
    let longest = document.width.max(document.height) as f32;
    let scale = max_size
        .filter(|size| *size > 0)
        .map_or(1.0, |size| (size as f32 / longest).min(1.0));
    render_document_scaled(document, scale)
}

/// Renders a complete document at an explicit canvas-pixel scale. Interactive
/// offscreen clients use this to match export semantics at physical display
/// resolution, including scales above 1 for editable parametric geometry.
pub fn render_document_scaled(document: &Document, scale: f32) -> Result<DynamicImage> {
    if !scale.is_finite() || scale <= 0.0 {
        bail!("document render scale must be a positive finite number");
    }
    let canvas_width = (document.width as f32 * scale).round().max(1.0) as u32;
    let canvas_height = (document.height as f32 * scale).round().max(1.0) as u32;
    if canvas_width > crate::MAX_CANVAS_DIMENSION || canvas_height > crate::MAX_CANVAS_DIMENSION {
        bail!("scaled document exceeds Prism's maximum canvas dimension");
    }
    let mut canvas = RgbaImage::from_pixel(canvas_width, canvas_height, Rgba(document.background));
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
        let source = render_layer_preview_scaled(&render_layer, None, shape_scale)?;
        let mut scaled_layer = layer.clone();
        scaled_layer.transform.x *= scale;
        scaled_layer.transform.y *= scale;
        scaled_layer.transform.scale_x *= scale / text_scale / shape_scale[0];
        scaled_layer.transform.scale_y *= scale / text_scale / shape_scale[1];
        let source = transform_layer(source, scaled_layer.transform);
        let mut coverage = RgbaImage::new(canvas_width, canvas_height);
        composite_layer(
            &mut canvas,
            &mut coverage,
            &source,
            &scaled_layer,
            previous_coverage.as_ref(),
        );
        previous_coverage = Some(coverage);
    }
    Ok(DynamicImage::ImageRgba8(canvas))
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

pub fn measure_text(text: &str, font_size: f32) -> Result<(u32, u32)> {
    let font = Font::from_bytes(
        epaint_default_fonts::UBUNTU_LIGHT,
        fontdue::FontSettings::default(),
    )
    .map_err(|error| anyhow::anyhow!("could not load bundled font: {error}"))?;
    let layout = layout_text(&font, text, font_size);
    Ok((layout.width, layout.height))
}

fn render_text(text: &str, font_size: f32, color: [u8; 4]) -> Result<RgbaImage> {
    let font = Font::from_bytes(
        epaint_default_fonts::UBUNTU_LIGHT,
        fontdue::FontSettings::default(),
    )
    .map_err(|error| anyhow::anyhow!("could not load bundled font: {error}"))?;
    let layout = layout_text(&font, text, font_size);
    let mut output = RgbaImage::new(layout.width, layout.height);
    for glyph in layout.glyphs {
        let (metrics, bitmap) = font.rasterize(glyph.character, font_size);
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let x = glyph.x + column as i32 - layout.min_x;
                let y = glyph.y + row as i32 - layout.min_y;
                if x >= 0 && y >= 0 && (x as u32) < layout.width && (y as u32) < layout.height {
                    let alpha = bitmap[row * metrics.width + column] as u16 * color[3] as u16 / 255;
                    output.put_pixel(
                        x as u32,
                        y as u32,
                        Rgba([color[0], color[1], color[2], alpha as u8]),
                    );
                }
            }
        }
    }
    Ok(output)
}

struct PositionedGlyph {
    character: char,
    x: i32,
    y: i32,
}

struct TextLayout {
    glyphs: Vec<PositionedGlyph>,
    min_x: i32,
    min_y: i32,
    width: u32,
    height: u32,
}

fn layout_text(font: &Font, text: &str, font_size: f32) -> TextLayout {
    let line_metrics = font.horizontal_line_metrics(font_size);
    let ascent = line_metrics.map_or(font_size, |metrics| metrics.ascent);
    let natural_height = line_metrics.map_or(font_size, |metrics| metrics.new_line_size);
    let line_height = natural_height.max(font_size * 1.25).ceil().max(1.0);
    let lines: Vec<_> = text.split('\n').collect();
    let mut glyphs = Vec::new();
    let mut min_x = 0;
    let mut min_y = 0;
    let mut max_x = 1;
    let mut max_y = (line_height * lines.len().max(1) as f32).ceil() as i32;
    for (line_index, line) in lines.iter().enumerate() {
        let mut cursor_x: f32 = 0.0;
        for character in line.chars() {
            let metrics = font.metrics(character, font_size);
            let x = cursor_x.round() as i32 + metrics.xmin;
            let baseline = line_index as f32 * line_height + ascent;
            let y = baseline.round() as i32 - metrics.ymin - metrics.height as i32;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + metrics.width as i32);
            max_y = max_y.max(y + metrics.height as i32);
            glyphs.push(PositionedGlyph { character, x, y });
            cursor_x += metrics.advance_width;
        }
        max_x = max_x.max(cursor_x.ceil() as i32);
    }
    TextLayout {
        glyphs,
        min_x,
        min_y,
        width: (max_x - min_x).max(1) as u32,
        height: (max_y - min_y).max(1) as u32,
    }
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

fn composite_layer(
    canvas: &mut RgbaImage,
    coverage: &mut RgbaImage,
    source: &RgbaImage,
    layer: &Layer,
    clip: Option<&RgbaImage>,
) {
    let origin_x = layer.transform.x.round() as i32;
    let origin_y = layer.transform.y.round() as i32;
    for (source_x, source_y, source_pixel) in source.enumerate_pixels() {
        let canvas_x = origin_x + source_x as i32;
        let canvas_y = origin_y + source_y as i32;
        if canvas_x < 0
            || canvas_y < 0
            || canvas_x >= canvas.width() as i32
            || canvas_y >= canvas.height() as i32
        {
            continue;
        }
        let normalized_x = source_x as f32 / source.width().max(1) as f32;
        let normalized_y = source_y as f32 / source.height().max(1) as f32;
        let in_mask = normalized_x >= layer.mask.x
            && normalized_x <= layer.mask.x + layer.mask.width
            && normalized_y >= layer.mask.y
            && normalized_y <= layer.mask.y + layer.mask.height;
        let mask_alpha = if !layer.mask.enabled || in_mask != layer.mask.invert {
            1.0
        } else {
            0.0
        };
        let x = canvas_x as u32;
        let y = canvas_y as u32;
        let clip_alpha = if layer.clip_to_below {
            clip.map_or(0.0, |image| image.get_pixel(x, y)[3] as f32 / 255.0)
        } else {
            1.0
        };
        let alpha = source_pixel[3] as f32 / 255.0 * layer.opacity * mask_alpha * clip_alpha;
        if alpha <= 0.0 {
            continue;
        }
        let destination = *canvas.get_pixel(x, y);
        let blended = blend_rgb(source_pixel.0, destination.0, layer.blend_mode);
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
