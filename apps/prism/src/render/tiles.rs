use anyhow::{Context, Result, bail};
use image::{DynamicImage, RgbaImage};

use crate::{Document, LayerKind, RasterSourceResolver, RegionRenderStats, RenderRegion};

use super::{render_document_region_scaled_untiled, scaled_document_dimensions};

pub(super) fn render_document_region_scaled_impl(
    document: &Document,
    scale: f32,
    region: RenderRegion,
    bound_fallback_layers: bool,
    raster_sources: Option<&dyn RasterSourceResolver>,
    stats: &mut RegionRenderStats,
) -> Result<DynamicImage> {
    validate_render_region(document, scale, region, bound_fallback_layers)?;
    let Some(tile_size) = paint_output_tile_size(document, scale) else {
        return render_document_region_scaled_untiled(
            document,
            scale,
            region,
            bound_fallback_layers,
            raster_sources,
            stats,
        );
    };
    if region.width <= tile_size && region.height <= tile_size {
        let (image, tile_stats) = render_document_tile_adaptive(
            document,
            scale,
            region,
            bound_fallback_layers,
            raster_sources,
        )?;
        stats.output_pixels = u64::from(region.width) * u64::from(region.height);
        merge_region_stats(stats, tile_stats);
        return Ok(DynamicImage::ImageRgba8(image));
    }
    let mut output = RgbaImage::new(region.width, region.height);
    stats.output_pixels = u64::from(region.width) * u64::from(region.height);
    for offset_y in (0..region.height).step_by(tile_size as usize) {
        for offset_x in (0..region.width).step_by(tile_size as usize) {
            let tile_region = RenderRegion {
                x: region.x + offset_x,
                y: region.y + offset_y,
                width: tile_size.min(region.width - offset_x),
                height: tile_size.min(region.height - offset_y),
            };
            let (tile, tile_stats) = render_document_tile_adaptive(
                document,
                scale,
                tile_region,
                bound_fallback_layers,
                raster_sources,
            )?;
            image::imageops::replace(&mut output, &tile, i64::from(offset_x), i64::from(offset_y));
            merge_region_stats(stats, tile_stats);
        }
    }
    Ok(DynamicImage::ImageRgba8(output))
}

fn validate_render_region(
    document: &Document,
    scale: f32,
    region: RenderRegion,
    bound_fallback_layers: bool,
) -> Result<()> {
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
    Ok(())
}

fn render_document_tile_adaptive(
    document: &Document,
    scale: f32,
    region: RenderRegion,
    bound_fallback_layers: bool,
    raster_sources: Option<&dyn RasterSourceResolver>,
) -> Result<(RgbaImage, RegionRenderStats)> {
    let mut stats = RegionRenderStats::default();
    match render_document_region_scaled_untiled(
        document,
        scale,
        region,
        bound_fallback_layers,
        raster_sources,
        &mut stats,
    ) {
        Ok(image) => Ok((image.to_rgba8(), stats)),
        Err(error)
            if (region.width > 1 || region.height > 1)
                && error.chain().any(|cause| {
                    cause
                        .downcast_ref::<crate::render_region::SourceStagingBudgetExceeded>()
                        .is_some()
                }) =>
        {
            let split_x = region.width >= region.height && region.width > 1;
            let first_length = if split_x {
                region.width / 2
            } else {
                region.height / 2
            };
            let first_region = RenderRegion {
                width: if split_x { first_length } else { region.width },
                height: if split_x { region.height } else { first_length },
                ..region
            };
            let second_region = RenderRegion {
                x: if split_x {
                    region.x + first_length
                } else {
                    region.x
                },
                y: if split_x {
                    region.y
                } else {
                    region.y + first_length
                },
                width: if split_x {
                    region.width - first_length
                } else {
                    region.width
                },
                height: if split_x {
                    region.height
                } else {
                    region.height - first_length
                },
            };
            let (first, first_stats) = render_document_tile_adaptive(
                document,
                scale,
                first_region,
                bound_fallback_layers,
                raster_sources,
            )?;
            let (second, second_stats) = render_document_tile_adaptive(
                document,
                scale,
                second_region,
                bound_fallback_layers,
                raster_sources,
            )?;
            let mut output = RgbaImage::new(region.width, region.height);
            image::imageops::replace(&mut output, &first, 0, 0);
            image::imageops::replace(
                &mut output,
                &second,
                if split_x { i64::from(first_length) } else { 0 },
                if split_x { 0 } else { i64::from(first_length) },
            );
            let mut combined = RegionRenderStats::default();
            merge_region_stats(&mut combined, first_stats);
            merge_region_stats(&mut combined, second_stats);
            Ok((output, combined))
        }
        Err(error) => Err(error),
    }
}

fn paint_output_tile_size(document: &Document, scale: f32) -> Option<u32> {
    document
        .layers
        .iter()
        .filter(|layer| {
            layer.visible && layer.opacity > 0.0 && matches!(layer.kind, LayerKind::Paint { .. })
        })
        .map(|layer| {
            let source_per_output_x = 1.0 / (layer.transform.scale_x.abs() * scale).max(0.0001);
            let source_per_output_y = 1.0 / (layer.transform.scale_y.abs() * scale).max(0.0001);
            let source_per_output = source_per_output_x.max(source_per_output_y);
            (4_000.0 / source_per_output).floor().clamp(1.0, 512.0) as u32
        })
        .min()
}

fn merge_region_stats(total: &mut RegionRenderStats, tile: RegionRenderStats) {
    total.source_staging_pixels = total
        .source_staging_pixels
        .saturating_add(tile.source_staging_pixels);
    total.source_staging_bytes = total
        .source_staging_bytes
        .saturating_add(tile.source_staging_bytes);
    total.max_source_staging_pixels = total
        .max_source_staging_pixels
        .max(tile.max_source_staging_pixels);
    total.adjusted_staging_pixels = total
        .adjusted_staging_pixels
        .saturating_add(tile.adjusted_staging_pixels);
    total.max_adjusted_staging_pixels = total
        .max_adjusted_staging_pixels
        .max(tile.max_adjusted_staging_pixels);
    total.full_source_pixels = total
        .full_source_pixels
        .saturating_add(tile.full_source_pixels);
    total.fallback_decode_bytes = total
        .fallback_decode_bytes
        .saturating_add(tile.fallback_decode_bytes);
    total.fallback_peak_bytes = total.fallback_peak_bytes.max(tile.fallback_peak_bytes);
    total.transformed_surface_pixels = total
        .transformed_surface_pixels
        .saturating_add(tile.transformed_surface_pixels);
    total.shadow_samples = total.shadow_samples.saturating_add(tile.shadow_samples);
    total.shadow_alpha_tile_pixels = total
        .shadow_alpha_tile_pixels
        .saturating_add(tile.shadow_alpha_tile_pixels);
    total.shadow_alpha_tile_bytes = total
        .shadow_alpha_tile_bytes
        .saturating_add(tile.shadow_alpha_tile_bytes);
    total.max_shadow_alpha_tile_pixels = total
        .max_shadow_alpha_tile_pixels
        .max(tile.max_shadow_alpha_tile_pixels);
    total.max_shadow_alpha_tile_bytes = total
        .max_shadow_alpha_tile_bytes
        .max(tile.max_shadow_alpha_tile_bytes);
}
