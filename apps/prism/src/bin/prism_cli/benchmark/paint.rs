use std::{hint::black_box, sync::mpsc, time::Instant};

use anyhow::{Result, bail};
use eframe::egui;
use prism_core::{
    BrushMode, BrushProgram, BrushSample, BrushStroke, BrushStyle, Command, Document, Layer,
    LayerKind, MAX_BRUSH_SAMPLES_PER_STROKE, MAX_PAINT_REGION_PIXELS, PaintSelection, RenderRegion,
    Workspace, preview_paint_command, render_document_region_scaled_with_stats,
};

const LIVE_BRUSH_FRAMES: usize = 24;
const LIVE_BRUSH_SAMPLES_PER_FRAME: usize = 4;
const LIVE_BRUSH_WORKERS: usize = 2;
const LIVE_BRUSH_MEMORY_BUDGET_BYTES: u64 = 16 * 1_024 * 1_024;

pub(super) struct PaintMeasurements {
    pub median_ms: f64,
    pub p95_ms: f64,
    pub max_source_staging_pixels: u64,
    pub drag_preview_median_ms: f64,
    pub drag_preview_p95_ms: f64,
    pub drag_preview_max_source_staging_pixels: u64,
    pub drag_preview_peak_bytes: u64,
    pub drag_preview_visible_pixels: u64,
}

pub(super) fn measure() -> Result<PaintMeasurements> {
    let drag_preview = measure_drag_preview()?;
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
        drag_preview_median_ms: drag_preview.median_ms,
        drag_preview_p95_ms: drag_preview.p95_ms,
        drag_preview_max_source_staging_pixels: drag_preview.max_source_staging_pixels,
        drag_preview_peak_bytes: drag_preview.peak_bytes,
        drag_preview_visible_pixels: drag_preview.visible_pixels,
    })
}

struct LiveBrushMeasurement {
    median_ms: f64,
    p95_ms: f64,
    max_source_staging_pixels: u64,
    peak_bytes: u64,
    visible_pixels: u64,
}

struct LiveRenderWork {
    worker: usize,
    sample_count: usize,
    started: Instant,
    document: Document,
    region: RenderRegion,
}

struct LiveRenderResult {
    worker: usize,
    sample_count: usize,
    started: Instant,
    rendered: image::DynamicImage,
    max_source_staging_pixels: u64,
}

fn live_brush_samples(sample_count: usize) -> Vec<BrushSample> {
    (0..sample_count)
        .map(|index| BrushSample {
            x: 7_040.0 + index as f32 * 5.0,
            y: 8_000.0 + (index as f32 * 0.19).sin() * 84.0,
            pressure: 0.55 + index as f32 / sample_count.max(1) as f32 * 0.45,
        })
        .collect()
}

fn live_brush_command(sample_count: usize) -> Result<Command> {
    Ok(Command::AddBrushStroke {
        id: 1,
        stroke: BrushStroke::new(
            BrushStyle {
                mode: BrushMode::Paint,
                color: [83, 203, 226, 230],
                size: 36.0,
                hardness: 0.75,
                opacity: 0.9,
                spacing: 0.15,
            },
            live_brush_samples(sample_count),
        )?,
        selection: PaintSelection::Current,
    })
}

fn build_live_work(
    worker: usize,
    frame: usize,
    base: &Document,
    region: RenderRegion,
) -> Result<LiveRenderWork> {
    let started = Instant::now();
    let sample_count = (frame + 1) * LIVE_BRUSH_SAMPLES_PER_FRAME;
    let document = preview_paint_command(base, live_brush_command(sample_count)?)?;
    let LayerKind::Paint { program } = &document.layer(1)?.kind else {
        bail!("live Brush preview changed its target layer kind");
    };
    if program.strokes.len() != 1 || program.strokes[0].samples.len() != sample_count {
        bail!("live Brush preview did not preserve its increasing sample prefix");
    }
    Ok(LiveRenderWork {
        worker,
        sample_count,
        started,
        document,
        region,
    })
}

fn render_live_work(work: LiveRenderWork) -> Result<LiveRenderResult> {
    let (rendered, stats) =
        render_document_region_scaled_with_stats(&work.document, 1.0, work.region)?;
    if (rendered.width(), rendered.height()) != (work.region.width, work.region.height) {
        bail!("live Brush worker returned unexpected output dimensions");
    }
    Ok(LiveRenderResult {
        worker: work.worker,
        sample_count: work.sample_count,
        started: work.started,
        rendered,
        max_source_staging_pixels: stats.max_source_staging_pixels,
    })
}

fn measure_drag_preview() -> Result<LiveBrushMeasurement> {
    let program = BrushProgram::new(16_384, 16_384)?;
    let mut base = Document::new("Live Brush preview benchmark", 16_384, 16_384);
    base.background = [0; 4];
    base.layers.push(Layer {
        id: 1,
        kind: LayerKind::Paint { program },
        ..Layer::default()
    });
    base.selected = Some(1);
    base.next_id = 2;
    let region = RenderRegion {
        x: 7_000,
        y: 7_800,
        width: 640,
        height: 400,
    };
    let output_pixels = u64::from(region.width) * u64::from(region.height);
    let frame_residency_bytes = output_pixels * 8;
    let peak_bytes = frame_residency_bytes * (LIVE_BRUSH_WORKERS as u64 + 1)
        + (LIVE_BRUSH_WORKERS
            * LIVE_BRUSH_FRAMES
            * LIVE_BRUSH_SAMPLES_PER_FRAME
            * std::mem::size_of::<BrushSample>()) as u64;
    if peak_bytes > LIVE_BRUSH_MEMORY_BUDGET_BYTES {
        bail!("live Brush benchmark exceeds its explicit UI/worker memory bound");
    }

    let mut timings = Vec::with_capacity(LIVE_BRUSH_FRAMES + 1);
    let mut visible_prefixes = Vec::with_capacity(LIVE_BRUSH_FRAMES);
    let mut max_source_staging_pixels = 0;
    let mut final_preview_pixels = None;
    std::thread::scope(|scope| -> Result<()> {
        let (result_sender, result_receiver) = mpsc::channel();
        let mut senders = Vec::with_capacity(LIVE_BRUSH_WORKERS);
        for _ in 0..LIVE_BRUSH_WORKERS {
            let (sender, receiver) = mpsc::sync_channel::<LiveRenderWork>(1);
            let result_sender = result_sender.clone();
            scope.spawn(move || {
                while let Ok(work) = receiver.recv() {
                    if result_sender.send(render_live_work(work)).is_err() {
                        break;
                    }
                }
            });
            senders.push(sender);
        }
        drop(result_sender);

        let mut next_frame = 0;
        let mut active = 0;
        let mut largest_scheduled_prefix = 0;
        for (worker, sender) in senders.iter().enumerate() {
            let work = build_live_work(worker, next_frame, &base, region)?;
            largest_scheduled_prefix = largest_scheduled_prefix.max(work.sample_count);
            sender
                .send(work)
                .map_err(|_| anyhow::anyhow!("live Brush benchmark worker stopped"))?;
            next_frame += 1;
            active += 1;
        }

        while active > 0 {
            let result = result_receiver
                .recv()
                .map_err(|_| anyhow::anyhow!("live Brush benchmark result channel stopped"))??;
            active -= 1;
            if result.sample_count > largest_scheduled_prefix {
                bail!("live Brush worker returned a prefix newer than the desired gesture");
            }
            if result.max_source_staging_pixels == 0
                || result.max_source_staging_pixels > MAX_PAINT_REGION_PIXELS
            {
                bail!("live Brush worker violated its source staging bound");
            }
            max_source_staging_pixels =
                max_source_staging_pixels.max(result.max_source_staging_pixels);
            let rgba = result.rendered.into_rgba8();
            let visible = rgba.pixels().filter(|pixel| pixel[3] != 0).count() as u64;
            let upload = egui::ColorImage::from_rgba_unmultiplied(
                [rgba.width() as usize, rgba.height() as usize],
                rgba.as_raw(),
            );
            timings.push(result.started.elapsed().as_secs_f64() * 1_000.0);
            visible_prefixes.push((result.sample_count, visible));
            if result.sample_count == LIVE_BRUSH_FRAMES * LIVE_BRUSH_SAMPLES_PER_FRAME {
                final_preview_pixels = Some(rgba.into_raw());
            }
            black_box(upload);

            if next_frame < LIVE_BRUSH_FRAMES {
                let work = build_live_work(result.worker, next_frame, &base, region)?;
                largest_scheduled_prefix = largest_scheduled_prefix.max(work.sample_count);
                senders[result.worker]
                    .send(work)
                    .map_err(|_| anyhow::anyhow!("live Brush benchmark worker stopped"))?;
                next_frame += 1;
                active += 1;
            }
        }
        drop(senders);
        Ok(())
    })?;

    visible_prefixes.sort_by_key(|(sample_count, _)| *sample_count);
    if visible_prefixes
        .windows(2)
        .any(|prefixes| prefixes[1].1 <= prefixes[0].1)
    {
        bail!("live Brush prefixes did not reveal strictly increasing visible pixels");
    }

    let settle_started = Instant::now();
    let final_command = live_brush_command(LIVE_BRUSH_FRAMES * LIVE_BRUSH_SAMPLES_PER_FRAME)?;
    let mut workspace = Workspace::new(base.clone(), None);
    workspace.execute(final_command)?;
    let (settled, settled_stats) =
        render_document_region_scaled_with_stats(&workspace.document, 1.0, region)?;
    let settled_rgba = settled.into_rgba8();
    let settled_upload = egui::ColorImage::from_rgba_unmultiplied(
        [
            settled_rgba.width() as usize,
            settled_rgba.height() as usize,
        ],
        settled_rgba.as_raw(),
    );
    timings.push(settle_started.elapsed().as_secs_f64() * 1_000.0);
    max_source_staging_pixels =
        max_source_staging_pixels.max(settled_stats.max_source_staging_pixels);
    if final_preview_pixels.as_deref() != Some(settled_rgba.as_raw()) {
        bail!("live Brush release jumped before the exact durable pixels settled");
    }
    black_box(settled_upload);

    if !matches!(
        &base.layer(1)?.kind,
        LayerKind::Paint { program } if program.strokes.is_empty()
    ) {
        bail!("Brush drag preview mutated the durable source document");
    }
    let visible_pixels = visible_prefixes
        .last()
        .map(|(_, visible)| *visible)
        .unwrap_or_default();
    let (median_ms, p95_ms) = super::sample_summary(&mut timings);
    Ok(LiveBrushMeasurement {
        median_ms,
        p95_ms,
        max_source_staging_pixels,
        peak_bytes,
        visible_pixels,
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
        assert!(measured.drag_preview_p95_ms >= measured.drag_preview_median_ms);
        assert!(measured.drag_preview_max_source_staging_pixels <= MAX_PAINT_REGION_PIXELS);
        assert!(measured.drag_preview_peak_bytes <= LIVE_BRUSH_MEMORY_BUDGET_BYTES);
        assert!(measured.drag_preview_visible_pixels > 0);
    }
}
