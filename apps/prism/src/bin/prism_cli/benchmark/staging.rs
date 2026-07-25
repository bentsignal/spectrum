use anyhow::Result;
use prism_core::{Document, Layer, RenderRegion, region_source_scales};

// spectrum-imaging expands adjusted regions by four source pixels for denoise
// and two more for sharpening.
const DEVELOPMENT_FILTER_HALO: f64 = 6.0;

pub(super) fn bounded_staging_budget(
    document: &Document,
    document_scale: f32,
    region: RenderRegion,
) -> Result<u64> {
    document
        .layers
        .iter()
        .filter(|layer| layer.visible && layer.opacity > 0.0)
        .map(|layer| layer_staging_budget(document, layer, document_scale, region))
        .try_fold(0, |maximum, budget| -> Result<u64> {
            Ok(maximum.max(budget?))
        })
}

fn layer_staging_budget(
    document: &Document,
    layer: &Layer,
    document_scale: f32,
    region: RenderRegion,
) -> Result<u64> {
    let scales = region_source_scales(document, layer, document_scale)?;
    let (rotated_width, rotated_height) = inverse_rotation_aabb(
        f64::from(region.width),
        f64::from(region.height),
        layer.transform.rotation,
    );
    let adjusted_width = triangle_source_extent(rotated_width, scales.outer_transform[0]);
    let adjusted_height = triangle_source_extent(rotated_height, scales.outer_transform[1]);
    let adjusted_pixels = adjusted_width.saturating_mul(adjusted_height);

    let halo = DEVELOPMENT_FILTER_HALO * 2.0;
    let (source_width, source_height) = inverse_rotation_aabb(
        adjusted_width as f64 + halo,
        adjusted_height as f64 + halo,
        layer.adjustments.straighten,
    );
    let bilinear_support = u64::from(layer.adjustments.straighten.abs() > 0.01) * 2;
    let source_width = source_width.ceil() as u64 + bilinear_support;
    let source_height = source_height.ceil() as u64 + bilinear_support;
    let source_pixels = source_width.saturating_mul(source_height);
    Ok(adjusted_pixels.max(source_pixels))
}

pub(super) fn inverse_rotation_aabb(width: f64, height: f64, degrees: f32) -> (f64, f64) {
    let radians = f64::from(degrees).to_radians();
    let (sin, cos) = radians.sin_cos();
    (
        width * cos.abs() + height * sin.abs(),
        width * sin.abs() + height * cos.abs(),
    )
}

pub(super) fn triangle_source_extent(output_extent: f64, outer_scale: f32) -> u64 {
    let inverse_scale = 1.0 / f64::from(outer_scale.abs().max(f32::EPSILON));
    let filter_radius = inverse_scale.max(1.0);
    (output_extent * inverse_scale + filter_radius * 2.0 + 2.0).ceil() as u64
}
