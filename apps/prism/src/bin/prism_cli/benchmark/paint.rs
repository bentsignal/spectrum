use std::time::Instant;

use anyhow::{Result, bail};
use prism_core::{
    BrushMode, BrushProgram, BrushSample, BrushStroke, BrushStyle, Document, Layer, LayerKind,
    MAX_BRUSH_SAMPLES_PER_STROKE, MAX_PAINT_REGION_PIXELS, RenderRegion,
    render_document_region_scaled_with_stats,
};

pub(super) struct PaintMeasurements {
    pub median_ms: f64,
    pub p95_ms: f64,
    pub max_source_staging_pixels: u64,
}

pub(super) fn measure() -> Result<PaintMeasurements> {
    let samples = (0..MAX_BRUSH_SAMPLES_PER_STROKE)
        .map(|index| {
            let t = index as f32 / (MAX_BRUSH_SAMPLES_PER_STROKE - 1) as f32;
            BrushSample {
                x: 0.5 + t * 16_383.0,
                y: 0.5 + t * 16_383.0,
                pressure: 0.35 + t * 0.65,
            }
        })
        .collect::<Vec<_>>();
    let stroke = BrushStroke::new(
        BrushStyle {
            mode: BrushMode::Paint,
            color: [83, 203, 226, 230],
            size: 24.0,
            hardness: 0.75,
            opacity: 0.9,
            spacing: 0.15,
        },
        samples,
    )?;
    let program = BrushProgram::new(16_384, 16_384)?.append(stroke)?;
    let mut document = Document::new("Sparse Paint benchmark", 16_384, 16_384);
    document.layers.push(Layer {
        id: 1,
        kind: LayerKind::Paint { program },
        ..Layer::default()
    });
    let region = RenderRegion {
        x: 8_032,
        y: 8_102,
        width: 320,
        height: 180,
    };
    let mut samples = Vec::with_capacity(7);
    let mut max_source_staging_pixels = 0;
    for _ in 0..7 {
        let started = Instant::now();
        let (rendered, stats) = render_document_region_scaled_with_stats(&document, 1.0, region)?;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        if (rendered.width(), rendered.height()) != (region.width, region.height) {
            bail!("sparse Paint benchmark returned unexpected output dimensions");
        }
        if stats.max_source_staging_pixels > MAX_PAINT_REGION_PIXELS {
            bail!("sparse Paint benchmark exceeded its source staging limit");
        }
        max_source_staging_pixels = max_source_staging_pixels.max(stats.max_source_staging_pixels);
    }
    samples.sort_by(f64::total_cmp);
    Ok(PaintMeasurements {
        median_ms: samples[samples.len() / 2],
        p95_ms: samples[samples.len() - 1],
        max_source_staging_pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_paint_benchmark_is_bounded() {
        let measured = measure().unwrap();
        assert!(measured.max_source_staging_pixels <= MAX_PAINT_REGION_PIXELS);
        assert!(measured.p95_ms >= measured.median_ms);
    }
}
