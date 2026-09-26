use std::time::Instant;

use anyhow::{Result, bail};
use prism_core::{Document, RenderRegion, render_document_region_scaled_with_stats};
use spectrum_imaging::Adjustments;

use super::bounded_staging_budget;

pub(super) struct AdjustedVectorBenchmark {
    pub(super) samples: Vec<f64>,
    pub(super) max_shadow_alpha_tile_pixels: u64,
}

pub(super) fn measure(
    base_document: &Document,
    region: RenderRegion,
) -> Result<AdjustedVectorBenchmark> {
    let mut document = base_document.clone();
    document.layers[0].adjustments = Adjustments {
        exposure: 0.25,
        vignette: -16.0,
        noise_reduction: 12.0,
        sharpening: 9.0,
        straighten: 3.0,
        ..Default::default()
    };
    document.layers[1].adjustments = Adjustments {
        contrast: 10.0,
        rotation: 90,
        straighten: -2.5,
        ..Default::default()
    };
    let staging_budget = bounded_staging_budget(&document, 8.0, region)?;
    let mut samples = Vec::new();
    let mut max_shadow_alpha_tile_pixels = 0;
    for _ in 0..5 {
        let started = Instant::now();
        let (rendered, stats) = render_document_region_scaled_with_stats(&document, 8.0, region)?;
        if (rendered.width(), rendered.height()) != (region.width, region.height)
            || stats.adjusted_staging_pixels == 0
            || stats.max_source_staging_pixels > staging_budget
            || stats.max_adjusted_staging_pixels > staging_budget
            || stats.transformed_surface_pixels != 0
            || stats.shadow_source_samples >= stats.shadow_samples
            || stats.max_shadow_alpha_tile_pixels > 4_096 * 4_096
            || stats.max_shadow_alpha_tile_bytes != stats.max_shadow_alpha_tile_pixels
        {
            bail!("adjusted vector viewport compositor violated its allocation contract");
        }
        max_shadow_alpha_tile_pixels =
            max_shadow_alpha_tile_pixels.max(stats.max_shadow_alpha_tile_pixels);
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok(AdjustedVectorBenchmark {
        samples,
        max_shadow_alpha_tile_pixels,
    })
}
