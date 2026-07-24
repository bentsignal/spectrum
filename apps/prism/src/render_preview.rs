use anyhow::{Context, Result, bail};
use image::DynamicImage;

use crate::{Document, Layer, LayerKind, Transform, render_document_scaled};

pub(crate) fn render_paint_preview_exact(
    layer: &Layer,
    max_size: Option<u32>,
) -> Result<DynamicImage> {
    let LayerKind::Paint { program } = &layer.kind else {
        bail!("exact Paint preview requires a Paint layer");
    };
    let mut preview_layer = layer.clone();
    preview_layer.visible = true;
    preview_layer.opacity = 1.0;
    preview_layer.blend_mode = crate::BlendMode::Normal;
    preview_layer.transform = Transform::default();
    preview_layer.style = crate::LayerStyle::default();
    preview_layer.mask = crate::LayerMask::default();
    preview_layer.clip_to_below = false;
    let adjusted_dimensions = spectrum_imaging::adjusted_image_dimensions(
        program.width,
        program.height,
        &layer.adjustments,
    )
    .context("Paint preview has invalid adjusted dimensions")?;
    let mut document = Document::new(
        "Paint preview",
        adjusted_dimensions.0,
        adjusted_dimensions.1,
    );
    document.background = [0; 4];
    document.layers.push(preview_layer);
    let longest = adjusted_dimensions.0.max(adjusted_dimensions.1) as f32;
    let scale = max_size
        .filter(|size| *size > 0)
        .map_or(1.0, |size| (size as f32 / longest).min(1.0));
    render_document_scaled(&document, scale)
}
