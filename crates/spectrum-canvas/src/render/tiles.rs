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
        // Bands of rows rendered at once across cores; each band is a region
        // render, so the result is the same as one pass. Each band falls back
        // to smaller regions when its sources exceed the staging budget, as a
        // large photo shown scaled down needs.
        let bands = band_regions(region);
        let rendered = render_each(&bands, |band| {
            render_document_tile_adaptive(
                document,
                scale,
                *band,
                bound_fallback_layers,
                raster_sources,
            )
        })?;
        stats.output_pixels = u64::from(region.width) * u64::from(region.height);
        if rendered.len() == 1 {
            let (image, band_stats) = rendered.into_iter().next().expect("one band");
            merge_region_stats(stats, band_stats);
            return Ok(DynamicImage::ImageRgba8(image));
        }
        let mut output = RgbaImage::new(region.width, region.height);
        for (band, (image, band_stats)) in bands.iter().zip(rendered) {
            image::imageops::replace(&mut output, &image, 0, i64::from(band.y - region.y));
            merge_region_stats(stats, band_stats);
        }
        return Ok(DynamicImage::ImageRgba8(output));
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
    let tiles: Vec<(u32, u32, RenderRegion)> = (0..region.height)
        .step_by(tile_size as usize)
        .flat_map(|offset_y| {
            (0..region.width)
                .step_by(tile_size as usize)
                .map(move |offset_x| {
                    let tile_region = RenderRegion {
                        x: region.x + offset_x,
                        y: region.y + offset_y,
                        width: tile_size.min(region.width - offset_x),
                        height: tile_size.min(region.height - offset_y),
                    };
                    (offset_x, offset_y, tile_region)
                })
        })
        .collect();
    // Tiles render at once across cores.
    let rendered = render_each(&tiles, |(_, _, tile_region)| {
        render_document_tile_adaptive(
            document,
            scale,
            *tile_region,
            bound_fallback_layers,
            raster_sources,
        )
    })?;
    for ((offset_x, offset_y, _), (tile, tile_stats)) in tiles.iter().zip(rendered) {
        image::imageops::replace(
            &mut output,
            &tile,
            i64::from(*offset_x),
            i64::from(*offset_y),
        );
        merge_region_stats(stats, tile_stats);
    }
    Ok(DynamicImage::ImageRgba8(output))
}

/// `render` for every item, on threads of their own, in order. Not on the
/// shared pool: a band waits for another band's work (one makes a reduced
/// copy of a photo while the rest wait for it), and a pool thread waiting
/// inside a lock picks up other bands, which then wait on themselves.
fn render_each<T: Sync, R: Send>(
    items: &[T],
    render: impl Fn(&T) -> Result<R> + Sync,
) -> Result<Vec<R>> {
    if items.len() <= 1 {
        return items.iter().map(&render).collect();
    }
    let threads = std::thread::available_parallelism()
        .map_or(4, |cores| cores.get())
        .min(items.len());
    let render = &render;
    let mut results: Vec<Option<Result<R>>> = (0..items.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|first| {
                scope.spawn(move || {
                    (first..items.len())
                        .step_by(threads)
                        .map(|index| (index, render(&items[index])))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for worker in workers {
            let done = worker
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            for (index, result) in done {
                results[index] = Some(result);
            }
        }
    });
    results
        .into_iter()
        .map(|result| result.expect("every item renders"))
        .collect()
}

/// Rows rendered as one band: enough that per-band work stays small.
const MIN_BAND_ROWS: u32 = 64;

/// Renders smaller than this many pixels go in one pass: starting threads
/// costs more than bands save, as for a text layer re-rendered as a slider
/// moves.
const MIN_BANDED_PIXELS: u64 = 512 * 512;

/// `region` cut into bands of rows, one per core or fewer.
fn band_regions(region: RenderRegion) -> Vec<RenderRegion> {
    if u64::from(region.width) * u64::from(region.height) < MIN_BANDED_PIXELS {
        return vec![region];
    }
    let cores = rayon::current_num_threads().max(1) as u32;
    let rows = region.height.div_ceil(cores).max(MIN_BAND_ROWS);
    (0..region.height)
        .step_by(rows as usize)
        .map(|offset| RenderRegion {
            y: region.y + offset,
            height: rows.min(region.height - offset),
            ..region
        })
        .collect()
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
        bail!("document render region exceeds Spectrum's maximum canvas dimension");
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
    total.shadow_source_samples = total
        .shadow_source_samples
        .saturating_add(tile.shadow_source_samples);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlendMode, Command, DropShadow, Glow, LayerStyle, Workspace};

    /// Rendering bands of rows at once gives exactly what one pass gives.
    #[test]
    fn bands_render_exactly_as_one_pass() {
        let mut workspace = Workspace::new(Document::new("Bands", 320, 520));
        let commands = vec![
            Command::AddRectangle {
                name: None,
                width: 260,
                height: 300,
                color: [40, 120, 200, 255],
                corner_radius: 18.0,
                x: 30.0,
                y: 40.0,
            },
            Command::AddText {
                text: "Bands of rows".into(),
                name: None,
                font_size: 42.0,
                color: [255, 255, 255, 255],
                x: 20.0,
                y: 150.0,
                shaping: Default::default(),
            },
            Command::AddEllipse {
                name: None,
                width: 200,
                height: 160,
                color: [250, 180, 60, 255],
                x: 60.0,
                y: 300.0,
            },
        ];
        workspace.execute_batch(commands).unwrap();
        let ids: Vec<u64> = workspace.document.layers.iter().map(|l| l.id).collect();
        workspace
            .execute_batch(vec![
                Command::SetLayerStyle {
                    id: ids[1],
                    style: LayerStyle {
                        drop_shadow: Some(DropShadow::default()),
                        outer_glow: Some(Glow {
                            size: 14.0,
                            ..Glow::default()
                        }),
                        ..LayerStyle::default()
                    },
                },
                Command::SetBlendMode {
                    id: ids[2],
                    blend_mode: BlendMode::Multiply,
                },
            ])
            .unwrap();
        let document = workspace.document;
        for scale in [0.75, 1.0, 1.6] {
            let (width, height) = scaled_document_dimensions(&document, scale).unwrap();
            let whole = RenderRegion {
                x: 0,
                y: 0,
                width,
                height,
            };
            let mut stats = RegionRenderStats::default();
            let one_pass = render_document_region_scaled_untiled(
                &document, scale, whole, false, None, &mut stats,
            )
            .unwrap()
            .to_rgba8();
            for band in band_regions(whole) {
                let mut stats = RegionRenderStats::default();
                let rendered = render_document_region_scaled_untiled(
                    &document, scale, band, false, None, &mut stats,
                )
                .unwrap()
                .to_rgba8();
                let expected =
                    image::imageops::crop_imm(&one_pass, 0, band.y, band.width, band.height)
                        .to_image();
                assert!(rendered == expected, "band at row {} scale {scale}", band.y);
            }
        }
    }
}
