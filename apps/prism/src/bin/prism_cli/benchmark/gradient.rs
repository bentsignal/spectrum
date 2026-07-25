use std::time::Instant;

use anyhow::{Result, bail};
use prism_core::{
    Document, GradientKind, GradientSpread, GradientStop, Layer, LayerKind, RenderRegion,
    ShapeFill, ShapeGradient, render_document_region_scaled_with_stats,
};
use spectrum_imaging::GradientSampleStats;

use super::support::sample_summary;

const DOCUMENT_DIMENSION: u32 = 16_384;
const DOCUMENT_SCALE: f32 = 8.0;
const STOP_SEARCH_BOUND: u64 = 6;

pub(super) struct GradientBenchmark {
    pub(super) radial_small_median_ms: f64,
    pub(super) radial_small_p95_ms: f64,
    pub(super) radial_large_median_ms: f64,
    pub(super) radial_large_p95_ms: f64,
    pub(super) angle_small_median_ms: f64,
    pub(super) angle_small_p95_ms: f64,
    pub(super) angle_large_median_ms: f64,
    pub(super) angle_large_p95_ms: f64,
}

struct KindBenchmark {
    small_median_ms: f64,
    small_p95_ms: f64,
    large_median_ms: f64,
    large_p95_ms: f64,
}

pub(super) fn measure() -> Result<GradientBenchmark> {
    let radial = measure_kind(GradientKind::Radial)?;
    let angle = measure_kind(GradientKind::Angle)?;
    Ok(GradientBenchmark {
        radial_small_median_ms: radial.small_median_ms,
        radial_small_p95_ms: radial.small_p95_ms,
        radial_large_median_ms: radial.large_median_ms,
        radial_large_p95_ms: radial.large_p95_ms,
        angle_small_median_ms: angle.small_median_ms,
        angle_small_p95_ms: angle.small_p95_ms,
        angle_large_median_ms: angle.large_median_ms,
        angle_large_p95_ms: angle.large_p95_ms,
    })
}

fn measure_kind(kind: GradientKind) -> Result<KindBenchmark> {
    let gradient = adversarial_gradient(kind);
    let document = gradient_document(gradient.clone());
    let small = RenderRegion {
        x: 41_357,
        y: 29_113,
        width: 320,
        height: 180,
    };
    let large = RenderRegion {
        x: 67_891,
        y: 52_347,
        width: 640,
        height: 400,
    };
    verify_uneven_strips(&document, small)?;
    verify_uneven_strips(&document, large)?;
    verify_sampler_bounds(&gradient, small)?;
    verify_sampler_bounds(&gradient, large)?;
    let (small_median_ms, small_p95_ms) = measure_region(&document, small)?;
    let (large_median_ms, large_p95_ms) = measure_region(&document, large)?;
    Ok(KindBenchmark {
        small_median_ms,
        small_p95_ms,
        large_median_ms,
        large_p95_ms,
    })
}

fn adversarial_gradient(kind: GradientKind) -> ShapeGradient {
    ShapeGradient {
        kind,
        angle: 137.0,
        stops: (0..32)
            .map(|index| {
                GradientStop::new(
                    index as f32 / 31.0,
                    [
                        (index * 71 % 256) as u8,
                        (index * 43 % 256) as u8,
                        (index * 19 % 256) as u8,
                        (55 + index * 6) as u8,
                    ],
                )
            })
            .collect(),
        center: [0.37, 0.63],
        radius: 0.41,
        spread: GradientSpread::Reflect,
        offset: -0.173,
        extent: 0.731,
        ..ShapeGradient::default()
    }
}

fn gradient_document(gradient: ShapeGradient) -> Document {
    let mut document = Document::new(
        "Adversarial gradient",
        DOCUMENT_DIMENSION,
        DOCUMENT_DIMENSION,
    );
    document.layers.push(Layer {
        id: 1,
        shape_fill: Some(ShapeFill::Gradient(gradient)),
        kind: LayerKind::Rectangle {
            width: DOCUMENT_DIMENSION,
            height: DOCUMENT_DIMENSION,
            color: [255; 4],
            corner_radius: 0.0,
        },
        ..Layer::default()
    });
    document
}

fn measure_region(document: &Document, region: RenderRegion) -> Result<(f64, f64)> {
    verify_render_bounds(document, region)?;
    let mut samples = Vec::with_capacity(7);
    for _ in 0..7 {
        let started = Instant::now();
        verify_render_bounds(document, region)?;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok(sample_summary(&mut samples))
}

fn verify_render_bounds(document: &Document, region: RenderRegion) -> Result<()> {
    let (image, stats) =
        render_document_region_scaled_with_stats(document, DOCUMENT_SCALE, region)?;
    let expected_pixels = u64::from(region.width) * u64::from(region.height);
    if image.width() != region.width
        || image.height() != region.height
        || stats.output_pixels != expected_pixels
        || stats.source_staging_pixels != 0
        || stats.source_staging_bytes != 0
        || stats.full_source_pixels != u64::from(DOCUMENT_DIMENSION).pow(2)
        || stats.fallback_decode_bytes != 0
        || stats.fallback_peak_bytes != 0
        || stats.transformed_surface_pixels != 0
    {
        bail!(
            "modern gradient viewport violated its exact output or zero-staging bounds: \
             image={}x{}, output={}, source_pixels={}, source_bytes={}, full={}, \
             fallback_decode={}, fallback_peak={}, transformed={}",
            image.width(),
            image.height(),
            stats.output_pixels,
            stats.source_staging_pixels,
            stats.source_staging_bytes,
            stats.full_source_pixels,
            stats.fallback_decode_bytes,
            stats.fallback_peak_bytes,
            stats.transformed_surface_pixels,
        );
    }
    Ok(())
}

fn verify_uneven_strips(document: &Document, region: RenderRegion) -> Result<()> {
    let (full, _) = render_document_region_scaled_with_stats(document, DOCUMENT_SCALE, region)?;
    let full = full.to_rgba8();
    let widths = [1, 17, 3, 89, 2, 131, 7, 53, 197, 5, 211];
    let mut x = 0;
    let mut index = 0;
    while x < region.width {
        let width = widths[index % widths.len()].min(region.width - x);
        let strip_region = RenderRegion {
            x: region.x + x,
            y: region.y,
            width,
            height: region.height,
        };
        let (strip, stats) =
            render_document_region_scaled_with_stats(document, DOCUMENT_SCALE, strip_region)?;
        if strip.to_rgba8()
            != image::imageops::crop_imm(&full, x, 0, width, region.height).to_image()
            || stats.source_staging_pixels != 0
            || stats.transformed_surface_pixels != 0
        {
            bail!("modern gradient uneven-strip output or staging contract diverged");
        }
        x += width;
        index += 1;
    }
    Ok(())
}

fn verify_sampler_bounds(gradient: &ShapeGradient, region: RenderRegion) -> Result<()> {
    let sampler = gradient.sampler();
    let mut stats = GradientSampleStats::default();
    for y in 0..region.height {
        for x in 0..region.width {
            sampler.sample_in_box_with_stats(
                (region.x + x) as f32 / DOCUMENT_SCALE,
                (region.y + y) as f32 / DOCUMENT_SCALE,
                DOCUMENT_DIMENSION as f32,
                DOCUMENT_DIMENSION as f32,
                &mut stats,
            );
        }
    }
    let expected_samples = u64::from(region.width) * u64::from(region.height);
    if stats.samples != expected_samples
        || stats.stop_comparisons > expected_samples * STOP_SEARCH_BOUND
        || stats.temporary_bytes != 0
        || stats.source_copy_bytes != 0
    {
        bail!("modern gradient stop-search, temporary, or source-copy bound regressed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "release-only strict gradient benchmark calibration"]
    fn release_gradient_fixture_reports_all_four_workloads() {
        assert!(
            !cfg!(debug_assertions),
            "run this calibration in release mode"
        );
        let measured = measure().unwrap();
        eprintln!(
            "radial small {:.3}/{:.3} ms, radial large {:.3}/{:.3} ms, \
             angle small {:.3}/{:.3} ms, angle large {:.3}/{:.3} ms",
            measured.radial_small_median_ms,
            measured.radial_small_p95_ms,
            measured.radial_large_median_ms,
            measured.radial_large_p95_ms,
            measured.angle_small_median_ms,
            measured.angle_small_p95_ms,
            measured.angle_large_median_ms,
            measured.angle_large_p95_ms,
        );
    }
}
