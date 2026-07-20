use std::time::Instant;

use anyhow::{Result, bail};
use prism_core::{
    Command, Document, Transform, Workspace, render_document, render_layer_base_scaled,
    render_solid_color,
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
    let (command_median, command_p95) = sample_summary(&mut command_samples);
    let (interaction_median, interaction_p95) = sample_summary(&mut interaction_samples);
    let (text_interaction_median, text_interaction_p95) =
        sample_summary(&mut text_interaction_samples);
    let (shape_median, shape_p95) = sample_summary(&mut shape_preview_samples);
    let (render_median, render_p95) = sample_summary(&mut render_samples);
    let (scaled_shape_median, scaled_shape_p95) = sample_summary(&mut scaled_shape_samples);
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

fn sample_summary(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    let p95_index = ((samples.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
    (median, samples[p95_index.min(samples.len() - 1)])
}
