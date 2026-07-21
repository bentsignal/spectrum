use std::path::Path;

use anyhow::{Context, Result, bail};
use image::ImageDecoder;

use crate::{FontAsset, Layer, LayerKind, text_render::measure_text_with_typography};

pub(crate) const MAX_REGION_FALLBACK_PEAK_BYTES: u64 = 256 * 1_024 * 1_024;
const MAX_FALLBACK_PIXELS: u64 = 4_096 * 4_096;

#[derive(Clone, Copy)]
pub(crate) struct FallbackAllocation {
    pub(crate) source_bytes: u64,
    pub(crate) estimated_peak_bytes: u64,
    pub(crate) scaled_pixels: u64,
}

pub(crate) fn ensure_region_fallback_is_bounded(
    render_layer: &Layer,
    scaled_layer: &Layer,
    shape_scale: [f32; 2],
    font_asset: Option<&FontAsset>,
) -> Result<FallbackAllocation> {
    let (base_width, base_height, source_bytes_per_pixel) = match &render_layer.kind {
        LayerKind::Raster { path, .. } => inspect_raster_decode_layout(path)?,
        LayerKind::Text {
            text,
            font_size,
            typography,
            ..
        } => {
            let (width, height) =
                measure_text_with_typography(text, *font_size, typography, font_asset)?;
            (width, height, 4)
        }
        LayerKind::Rectangle { width, height, .. } | LayerKind::Ellipse { width, height, .. } => (
            (*width as f32 * shape_scale[0]).round().max(1.0) as u32,
            (*height as f32 * shape_scale[1]).round().max(1.0) as u32,
            4,
        ),
    };
    let (adjusted_width, adjusted_height) = spectrum_imaging::adjusted_image_dimensions(
        base_width,
        base_height,
        &render_layer.adjustments,
    )
    .context("fallback source has invalid adjusted dimensions")?;
    let scaled_width = (adjusted_width as f32 * scaled_layer.transform.scale_x)
        .abs()
        .round()
        .max(1.0) as u32;
    let scaled_height = (adjusted_height as f32 * scaled_layer.transform.scale_y)
        .abs()
        .round()
        .max(1.0) as u32;
    let (transformed_width, transformed_height) = if scaled_layer.transform.rotation.abs() < 0.01 {
        (scaled_width, scaled_height)
    } else {
        let (sin, cos) = crate::transform_math::rotation_sin_cos(scaled_layer.transform.rotation);
        (
            (scaled_width as f32 * cos.abs() + scaled_height as f32 * sin.abs())
                .ceil()
                .max(1.0) as u32,
            (scaled_width as f32 * sin.abs() + scaled_height as f32 * cos.abs())
                .ceil()
                .max(1.0) as u32,
        )
    };
    let base_pixels = checked_pixels(base_width, base_height, "fallback base area overflows")?;
    let adjusted_pixels = checked_pixels(
        adjusted_width,
        adjusted_height,
        "fallback adjusted area overflows",
    )?;
    let scaled_pixels = checked_pixels(
        scaled_width,
        scaled_height,
        "fallback scaled area overflows",
    )?;
    let transformed_pixels = checked_pixels(
        transformed_width,
        transformed_height,
        "fallback transformed area overflows",
    )?;
    let (source_bytes, peak_bytes) = estimate_fallback_peak_bytes(
        (base_width, base_height),
        source_bytes_per_pixel,
        (adjusted_width, adjusted_height),
        scaled_pixels,
        transformed_pixels,
        &render_layer.adjustments,
        scaled_layer.transform.rotation.abs() >= 0.01,
    )?;
    if base_width > crate::MAX_CANVAS_DIMENSION
        || base_height > crate::MAX_CANVAS_DIMENSION
        || adjusted_width > crate::MAX_CANVAS_DIMENSION
        || adjusted_height > crate::MAX_CANVAS_DIMENSION
        || transformed_width > crate::MAX_CANVAS_DIMENSION
        || transformed_height > crate::MAX_CANVAS_DIMENSION
        || base_pixels > MAX_FALLBACK_PIXELS
        || adjusted_pixels > MAX_FALLBACK_PIXELS
        || scaled_pixels > MAX_FALLBACK_PIXELS
        || transformed_pixels > MAX_FALLBACK_PIXELS
        || peak_bytes > MAX_REGION_FALLBACK_PEAK_BYTES
    {
        bail!(
            "layer {} exceeds the bounded viewport fallback; lower zoom or simplify the layer",
            render_layer.id
        );
    }
    Ok(FallbackAllocation {
        source_bytes,
        estimated_peak_bytes: peak_bytes,
        scaled_pixels,
    })
}

fn checked_pixels(width: u32, height: u32, message: &'static str) -> Result<u64> {
    u64::from(width)
        .checked_mul(u64::from(height))
        .context(message)
}

#[allow(clippy::too_many_arguments)]
fn estimate_fallback_peak_bytes(
    base_dimensions: (u32, u32),
    source_bytes_per_pixel: u64,
    adjusted_dimensions: (u32, u32),
    scaled_pixels: u64,
    transformed_pixels: u64,
    adjustments: &spectrum_imaging::Adjustments,
    outer_rotation: bool,
) -> Result<(u64, u64)> {
    // image::blur holds the RGBA input, two RGBA-f32 transients, and an RGBA
    // output concurrently. Its working/output buffers total eleven RGBA-sized
    // surfaces at peak, in addition to render_image's retained current image.
    const BLUR_RGBA_SURFACES: u64 = 11;
    let adjustments = adjustments.clone().sanitized();
    let base_pixels = checked_pixels(
        base_dimensions.0,
        base_dimensions.1,
        "fallback base area overflows",
    )?;
    let source_bytes = base_pixels
        .checked_mul(source_bytes_per_pixel)
        .context("fallback source byte count overflows")?;
    let mut current_bytes = source_bytes;
    let mut current_bytes_per_pixel = source_bytes_per_pixel;
    let mut peak_bytes = source_bytes;

    if adjustments.rotation != 0 {
        peak_bytes = peak_bytes.max(checked_multiple(current_bytes, 2, "rotation")?);
    }
    for enabled in [adjustments.flip_horizontal, adjustments.flip_vertical] {
        if enabled {
            peak_bytes = peak_bytes.max(checked_multiple(current_bytes, 2, "flip")?);
        }
    }
    if adjustments.straighten.abs() > 0.01 {
        let rgba_bytes = checked_multiple(base_pixels, 4, "straighten surface")?;
        peak_bytes = peak_bytes.max(
            current_bytes
                .checked_add(checked_multiple(rgba_bytes, 2, "straighten")?)
                .context("fallback straighten peak overflows")?,
        );
        current_bytes = rgba_bytes;
        current_bytes_per_pixel = 4;
    }

    let adjusted_pixels = checked_pixels(
        adjusted_dimensions.0,
        adjusted_dimensions.1,
        "fallback adjusted area overflows",
    )?;
    if adjustments.crop.is_some() {
        let cropped_bytes =
            checked_multiple(adjusted_pixels, current_bytes_per_pixel, "crop surface")?;
        peak_bytes = peak_bytes.max(
            current_bytes
                .checked_add(cropped_bytes)
                .context("fallback crop peak overflows")?,
        );
        current_bytes = cropped_bytes;
    }

    let has_pixel_adjustments =
        !adjustments.as_preset().is_identity() || !adjustments.spots.is_empty();
    if has_pixel_adjustments {
        let rgba_bytes = checked_multiple(adjusted_pixels, 4, "RGBA working surface")?;
        peak_bytes = peak_bytes.max(
            current_bytes
                .checked_add(rgba_bytes)
                .context("fallback RGBA conversion peak overflows")?,
        );
        let working_surfaces = if adjustments.noise_reduction > 0.0 || adjustments.sharpening > 0.0
        {
            BLUR_RGBA_SURFACES
        } else if !adjustments.spots.is_empty() {
            2
        } else {
            1
        };
        peak_bytes = peak_bytes.max(
            current_bytes
                .checked_add(checked_multiple(
                    rgba_bytes,
                    working_surfaces,
                    "adjustment",
                )?)
                .context("fallback adjustment peak overflows")?,
        );
        current_bytes = rgba_bytes;
        current_bytes_per_pixel = 4;
    }

    let scaled_native_bytes = checked_multiple(
        scaled_pixels,
        current_bytes_per_pixel,
        "scaled native surface",
    )?;
    let scaled_rgba_bytes = checked_multiple(scaled_pixels, 4, "scaled RGBA surface")?;
    peak_bytes = peak_bytes.max(
        current_bytes
            .checked_add(scaled_native_bytes)
            .and_then(|bytes| bytes.checked_add(scaled_rgba_bytes))
            .context("fallback resize peak overflows")?,
    );
    if outer_rotation {
        let rotated_bytes = checked_multiple(transformed_pixels, 4, "rotated surface")?;
        peak_bytes = peak_bytes.max(
            scaled_rgba_bytes
                .checked_add(rotated_bytes)
                .context("fallback outer rotation peak overflows")?,
        );
    }
    Ok((source_bytes, peak_bytes))
}

fn checked_multiple(value: u64, multiplier: u64, label: &str) -> Result<u64> {
    value
        .checked_mul(multiplier)
        .with_context(|| format!("fallback {label} peak overflows"))
}

fn inspect_raster_decode_layout(path: &Path) -> Result<(u32, u32, u64)> {
    let mut reader = image::ImageReader::open(path)
        .with_context(|| format!("could not open {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("could not inspect layer source {}", path.display()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(crate::MAX_CANVAS_DIMENSION);
    limits.max_image_height = Some(crate::MAX_CANVAS_DIMENSION);
    // max_alloc is non-strict for some codecs; the explicit operation-aware
    // estimator above is the primary guard and decoder limits are defense-in-depth.
    limits.max_alloc = Some(MAX_REGION_FALLBACK_PEAK_BYTES);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .with_context(|| format!("could not inspect layer source {}", path.display()))?;
    let (width, height) = decoder.dimensions();
    Ok((
        width,
        height,
        u64::from(decoder.color_type().bytes_per_pixel()),
    ))
}
