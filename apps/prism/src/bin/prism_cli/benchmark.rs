use std::{io::Write, path::PathBuf, time::Instant};

use anyhow::{Result, bail};
use prism_core::{
    BlendMode, Command, Document, Layer, LayerKind, LayerMask, RenderRegion, Transform, Workspace,
    render_document, render_document_region_scaled, render_document_region_scaled_with_stats,
    render_layer_base_scaled, render_solid_color,
};
use serde::Serialize;
use serde_json::{Value, json};
use spectrum_imaging::Adjustments;

#[derive(Serialize)]
struct BenchmarkMetric {
    name: &'static str,
    median_ms: f64,
    p95_ms: f64,
    budget_ms: f64,
    pass: bool,
}

pub(super) fn benchmark(strict: bool) -> Result<Value> {
    let mut command_samples = Vec::new();
    let mut workspace = None;
    for _ in 0..9 {
        let mut sample = Workspace::new(Document::new("Benchmark", 1600, 1200), None);
        let started = Instant::now();
        for index in 0..24 {
            sample.execute(Command::AddRectangle {
                name: Some(format!("Layer {index}")),
                width: 720,
                height: 480,
                color: [40 + index * 6, 90, 180, 180],
                corner_radius: 24.0,
                x: (index * 17) as f32,
                y: (index * 11) as f32,
            })?;
        }
        command_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        workspace = Some(sample);
    }
    let workspace = workspace.expect("benchmark always records at least one command sample");
    let mut interaction_workspace = Workspace::new(workspace.document.clone(), None);
    let interaction_layer = interaction_workspace.document.layers.last().unwrap().id;
    interaction_workspace.begin_interaction();
    let mut interaction_samples = Vec::new();
    for frame in 0..240 {
        let started = Instant::now();
        interaction_workspace.preview(Command::SetTransform {
            id: interaction_layer,
            transform: Transform {
                x: frame as f32 * 2.0,
                y: frame as f32,
                scale_x: 1.0 + frame as f32 / 1_000.0,
                scale_y: 1.0 + frame as f32 / 1_000.0,
                rotation: 0.0,
            },
        })?;
        interaction_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    interaction_workspace.commit_interaction()?;
    let mut text_workspace = Workspace::new(Document::new("Text benchmark", 1600, 1200), None);
    text_workspace.execute(Command::AddText {
        text: "Prism interaction benchmark".into(),
        name: Some("Text".into()),
        font_size: 144.0,
        color: [255, 255, 255, 255],
        x: 100.0,
        y: 100.0,
    })?;
    let text_layer = text_workspace.document.selected.unwrap();
    text_workspace.begin_interaction();
    let mut text_interaction_samples = Vec::new();
    for frame in 0..240 {
        let started = Instant::now();
        text_workspace.preview(Command::SetTransform {
            id: text_layer,
            transform: Transform {
                x: 100.0 + frame as f32 * 2.0,
                y: 100.0 + frame as f32,
                ..Default::default()
            },
        })?;
        text_interaction_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    text_workspace.commit_interaction()?;
    let mut shape_preview_samples = Vec::new();
    for frame in 0..240 {
        let adjustments = Adjustments {
            exposure: frame as f32 / 48.0 - 2.5,
            contrast: frame as f32 % 100.0 - 50.0,
            ..Default::default()
        };
        let started = Instant::now();
        let _ = render_solid_color([93, 216, 199, 255], &adjustments);
        shape_preview_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let mut render_samples = Vec::new();
    let mut rendered = None;
    for _ in 0..7 {
        let started = Instant::now();
        rendered = Some(render_document(&workspace.document, None)?);
        render_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let mut scaled_shape_workspace = Workspace::new(Document::new("Scaled shape", 800, 600), None);
    scaled_shape_workspace.execute(Command::AddEllipse {
        name: Some("Scale benchmark".into()),
        width: 32,
        height: 24,
        color: [93, 216, 199, 255],
        x: 0.0,
        y: 0.0,
    })?;
    let scaled_shape = scaled_shape_workspace.document.layer(1)?;
    let mut scaled_shape_samples = Vec::new();
    for _ in 0..9 {
        let started = Instant::now();
        let _ = render_layer_base_scaled(scaled_shape, None, [16.0, 16.0])?;
        scaled_shape_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let mut blend_workspace = Workspace::new(Document::new("Blend benchmark", 960, 540), None);
    for index in 0..12 {
        blend_workspace.execute(Command::AddRectangle {
            name: Some(format!("Blend {index}")),
            width: 640,
            height: 360,
            color: [45 + index * 14, 190 - index * 9, 80 + index * 8, 210],
            corner_radius: 28.0,
            x: (index * 23) as f32 - 80.0,
            y: (index * 17) as f32 - 60.0,
        })?;
        let id = blend_workspace.document.selected.unwrap();
        blend_workspace.execute(Command::SetBlendMode {
            id,
            blend_mode: BlendMode::ALL[(index as usize * 2 + 1) % BlendMode::ALL.len()],
        })?;
        if index % 3 == 1 {
            blend_workspace.execute(Command::SetClipping { id, enabled: true })?;
        }
        if index % 4 == 2 {
            blend_workspace.execute(Command::SetMask {
                id,
                mask: LayerMask {
                    enabled: true,
                    x: 0.15,
                    y: 0.1,
                    width: 0.7,
                    height: 0.8,
                    invert: index % 8 == 6,
                },
            })?;
        }
    }
    let mut blend_render_samples = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        let _ = render_document(&blend_workspace.document, None)?;
        blend_render_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let mut viewport_workspace = Workspace::new(Document::new("Viewport", 16_384, 16_384), None);
    for index in 0..6 {
        viewport_workspace.execute(Command::AddRectangle {
            name: Some(format!("Viewport blend {index}")),
            width: 16_384,
            height: 16_384,
            color: [60 + index * 20, 180 - index * 14, 100 + index * 17, 210],
            corner_radius: 0.0,
            x: 0.0,
            y: 0.0,
        })?;
        let id = viewport_workspace.document.selected.unwrap();
        viewport_workspace.execute(Command::SetBlendMode {
            id,
            blend_mode: BlendMode::ALL[5 + index as usize * 3],
        })?;
        if index == 3 {
            viewport_workspace.execute(Command::SetClipping { id, enabled: true })?;
        }
        if index == 4 {
            viewport_workspace.execute(Command::SetMask {
                id,
                mask: LayerMask {
                    enabled: true,
                    invert: true,
                    x: 0.2,
                    y: 0.2,
                    width: 0.6,
                    height: 0.6,
                },
            })?;
        }
    }
    let viewport_region = RenderRegion {
        x: 512,
        y: 448,
        width: 960,
        height: 540,
    };
    let mut viewport_composite_samples = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let rendered =
            render_document_region_scaled(&viewport_workspace.document, 8.0, viewport_region)?;
        if (rendered.width(), rendered.height()) != (viewport_region.width, viewport_region.height)
        {
            bail!("viewport compositor returned the wrong physical dimensions");
        }
        viewport_composite_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let large_raster = TemporaryRaster::new(16_384, 1_025)?;
    let mut bounded_sources = Document::new("Bounded sources", 16_384, 2_048);
    bounded_sources.layers = vec![
        Layer {
            id: 100,
            transform: Transform {
                x: 7_760.0,
                y: 260.0,
                scale_x: 1.15,
                scale_y: 1.1,
                rotation: 3.0,
            },
            kind: LayerKind::Raster {
                path: large_raster.path.clone(),
                original_path: None,
            },
            ..Layer::default()
        },
        Layer {
            id: 101,
            opacity: 0.78,
            blend_mode: BlendMode::Overlay,
            transform: Transform {
                x: 7_940.0,
                y: 380.0,
                rotation: 11.0,
                ..Transform::default()
            },
            kind: LayerKind::Text {
                text: "Bounded viewport text ".repeat(320),
                font_size: 48.0,
                color: [242, 207, 116, 230],
                typography: Default::default(),
            },
            ..Layer::default()
        },
    ];
    let bounded_region = RenderRegion {
        x: 8_000,
        y: 400,
        width: 960,
        height: 540,
    };
    let mut bounded_source_samples = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        let (rendered, stats) =
            render_document_region_scaled_with_stats(&bounded_sources, 1.0, bounded_region)?;
        if (rendered.width(), rendered.height()) != (bounded_region.width, bounded_region.height) {
            bail!("bounded source compositor returned the wrong physical dimensions");
        }
        if stats.full_source_pixels <= 4_096 * 4_096
            || stats.source_staging_pixels >= stats.full_source_pixels
            || stats.fallback_decode_bytes != 0
            || stats.transformed_surface_pixels != 0
        {
            bail!("bounded source compositor regressed to full-source staging");
        }
        bounded_source_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let (command_median, command_p95) = sample_summary(&mut command_samples);
    let (interaction_median, interaction_p95) = sample_summary(&mut interaction_samples);
    let (text_interaction_median, text_interaction_p95) =
        sample_summary(&mut text_interaction_samples);
    let (shape_median, shape_p95) = sample_summary(&mut shape_preview_samples);
    let (render_median, render_p95) = sample_summary(&mut render_samples);
    let (scaled_shape_median, scaled_shape_p95) = sample_summary(&mut scaled_shape_samples);
    let (blend_render_median, blend_render_p95) = sample_summary(&mut blend_render_samples);
    let (viewport_composite_median, viewport_composite_p95) =
        sample_summary(&mut viewport_composite_samples);
    let (bounded_source_median, bounded_source_p95) = sample_summary(&mut bounded_source_samples);
    let metrics = [
        BenchmarkMetric {
            name: "flat_shape_adjustment_preview",
            median_ms: shape_median,
            p95_ms: shape_p95,
            budget_ms: 0.5,
            pass: shape_p95 <= 0.5,
        },
        BenchmarkMetric {
            name: "live_transform_preview",
            median_ms: interaction_median,
            p95_ms: interaction_p95,
            budget_ms: 8.0,
            pass: interaction_p95 <= 8.0,
        },
        BenchmarkMetric {
            name: "live_text_move_preview",
            median_ms: text_interaction_median,
            p95_ms: text_interaction_p95,
            budget_ms: 1.0,
            pass: text_interaction_p95 <= 1.0,
        },
        BenchmarkMetric {
            name: "24_layer_command_batch",
            median_ms: command_median,
            p95_ms: command_p95,
            budget_ms: 50.0,
            pass: command_p95 <= 50.0,
        },
        BenchmarkMetric {
            name: "1600x1200_24_layer_composite",
            median_ms: render_median,
            p95_ms: render_p95,
            budget_ms: 2_000.0,
            pass: render_p95 <= 2_000.0,
        },
        BenchmarkMetric {
            name: "16x_parametric_shape_raster",
            median_ms: scaled_shape_median,
            p95_ms: scaled_shape_p95,
            budget_ms: 16.0,
            pass: scaled_shape_p95 <= 16.0,
        },
        BenchmarkMetric {
            name: "960x540_12_layer_blend_mask_composite",
            median_ms: blend_render_median,
            p95_ms: blend_render_p95,
            budget_ms: 500.0,
            pass: blend_render_p95 <= 500.0,
        },
        BenchmarkMetric {
            name: "8x_zoom_16k_document_viewport_composite",
            median_ms: viewport_composite_median,
            p95_ms: viewport_composite_p95,
            budget_ms: 500.0,
            pass: viewport_composite_p95 <= 500.0,
        },
        BenchmarkMetric {
            name: "large_rotated_raster_text_bounded_staging",
            median_ms: bounded_source_median,
            p95_ms: bounded_source_p95,
            budget_ms: 750.0,
            pass: bounded_source_p95 <= 750.0,
        },
    ];
    let passed = metrics.iter().all(|metric| metric.pass);
    if strict && !passed {
        bail!("Prism benchmark exceeded a strict regression budget");
    }
    Ok(json!({
        "ok": true,
        "action": "benchmark",
        "strict": strict,
        "passed": passed,
        "output": [rendered.as_ref().unwrap().width(), rendered.as_ref().unwrap().height()],
        "metrics": metrics
    }))
}

struct TemporaryRaster {
    path: PathBuf,
}

impl TemporaryRaster {
    fn new(width: u32, height: u32) -> Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("prism-benchmark-{stamp}.png"));
        let file = std::fs::File::create(&path)?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        let mut stream = writer.stream_writer()?;
        let mut row = vec![0; width as usize];
        for y in 0..height {
            for (x, pixel) in row.iter_mut().enumerate() {
                *pixel = ((x as u32 * 17 + y * 31) % 256) as u8;
            }
            stream.write_all(&row)?;
        }
        stream.finish()?;
        Ok(Self { path })
    }
}

impl Drop for TemporaryRaster {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn sample_summary(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    let p95_index = ((samples.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
    (median, samples[p95_index.min(samples.len() - 1)])
}
