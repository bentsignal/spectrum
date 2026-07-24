use std::time::Instant;

use anyhow::{Result, bail};
use prism_core::{
    BrushMode, BrushProgram, BrushSample, BrushStroke, BrushStyle, Command, Document, Layer,
    LayerKind, MAX_BRUSH_SAMPLES_PER_STROKE, MAX_PAINT_REGION_PIXELS, PaintSelection, RenderRegion,
    preview_paint_command, render_document_region_scaled_with_stats,
};

pub(super) struct PaintMeasurements {
    pub median_ms: f64,
    pub p95_ms: f64,
    pub max_source_staging_pixels: u64,
    pub drag_preview_median_ms: f64,
    pub drag_preview_p95_ms: f64,
}

pub(super) fn measure() -> Result<PaintMeasurements> {
    let (drag_preview_median_ms, drag_preview_p95_ms) = measure_drag_preview()?;
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
        drag_preview_median_ms,
        drag_preview_p95_ms,
    })
}

fn measure_drag_preview() -> Result<(f64, f64)> {
    let program = BrushProgram::new(16_384, 16_384)?;
    let mut base = Document::new("Live Brush preview benchmark", 16_384, 16_384);
    base.layers.push(Layer {
        id: 1,
        kind: LayerKind::Paint { program },
        ..Layer::default()
    });
    base.selected = Some(1);
    base.next_id = 2;
    let mut points = Vec::with_capacity(240);
    let mut timings = Vec::with_capacity(240);
    for frame in 0..240 {
        points.push(BrushSample {
            x: 7_200.0 + frame as f32 * 4.0,
            y: 8_000.0 + (frame as f32 * 0.13).sin() * 220.0,
            pressure: 1.0,
        });
        let started = Instant::now();
        let stroke = BrushStroke::new(
            BrushStyle {
                mode: BrushMode::Paint,
                color: [83, 203, 226, 230],
                size: 48.0,
                hardness: 0.75,
                opacity: 0.9,
                spacing: 0.15,
            },
            points.clone(),
        )?;
        let preview = preview_paint_command(
            &base,
            Command::AddBrushStroke {
                id: 1,
                stroke,
                selection: PaintSelection::Current,
            },
        )?;
        timings.push(started.elapsed().as_secs_f64() * 1_000.0);
        let LayerKind::Paint { program } = &preview.layer(1)?.kind else {
            bail!("Brush drag preview changed its target layer kind");
        };
        if program.strokes.len() != 1 || program.strokes[0].samples.len() != points.len() {
            bail!("Brush drag preview did not preserve the monotonic sample prefix");
        }
    }
    if !matches!(
        &base.layer(1)?.kind,
        LayerKind::Paint { program } if program.strokes.is_empty()
    ) {
        bail!("Brush drag preview mutated the durable source document");
    }
    timings.sort_by(f64::total_cmp);
    let median = timings[timings.len() / 2];
    let p95_index = ((timings.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(timings.len() - 1);
    Ok((median, timings[p95_index]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_paint_benchmark_is_bounded() {
        let measured = measure().unwrap();
        assert!(measured.max_source_staging_pixels <= MAX_PAINT_REGION_PIXELS);
        assert!(measured.p95_ms >= measured.median_ms);
        assert!(measured.drag_preview_p95_ms >= measured.drag_preview_median_ms);
    }
}
