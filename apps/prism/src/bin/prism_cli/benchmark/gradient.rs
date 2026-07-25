use std::time::Instant;

use anyhow::{Result, bail};
use prism_core::{
    Document, GradientKind, GradientSpread, GradientStop, Layer, LayerKind, RenderRegion,
    ShapeFill, ShapeGradient, render_document_region_scaled_with_stats,
};

use super::support::sample_summary;

pub(super) struct GradientBenchmark {
    pub(super) radial_median_ms: f64,
    pub(super) radial_p95_ms: f64,
    pub(super) angle_median_ms: f64,
    pub(super) angle_p95_ms: f64,
}

pub(super) fn measure() -> Result<GradientBenchmark> {
    let (radial_median_ms, radial_p95_ms) = measure_kind(GradientKind::Radial)?;
    let (angle_median_ms, angle_p95_ms) = measure_kind(GradientKind::Angle)?;
    Ok(GradientBenchmark {
        radial_median_ms,
        radial_p95_ms,
        angle_median_ms,
        angle_p95_ms,
    })
}

fn measure_kind(kind: GradientKind) -> Result<(f64, f64)> {
    let mut document = Document::new("Adversarial gradient", 16_384, 16_384);
    document.layers.push(Layer {
        id: 1,
        shape_fill: Some(ShapeFill::Gradient(ShapeGradient {
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
        })),
        kind: LayerKind::Rectangle {
            width: 16_384,
            height: 16_384,
            color: [255; 4],
            corner_radius: 0.0,
        },
        ..Layer::default()
    });
    let region = RenderRegion {
        x: 41_357,
        y: 29_113,
        width: 320,
        height: 180,
    };
    let mut samples = Vec::with_capacity(7);
    for _ in 0..7 {
        let started = Instant::now();
        let (image, stats) = render_document_region_scaled_with_stats(&document, 8.0, region)?;
        if image.width() != region.width
            || image.height() != region.height
            || stats.source_staging_pixels != 0
            || stats.transformed_surface_pixels != 0
        {
            bail!("modern gradient viewport regressed to full-source staging");
        }
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok(sample_summary(&mut samples))
}
