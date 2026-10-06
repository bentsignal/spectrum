//! Image rendering and editing performance: previews while adjusting, a
//! saved edit on a 24 MP image, and a full-size export.
use std::{
    fs,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, Rgb, RgbImage};
use serde_json::{Value, json};
use spectrum_document::{Actor, ActorKind, SessionId};
use spectrum_image::{
    Adjustments, Command, CurvePoint, Image, ToneCurve, ToneCurves, Workspace,
    engine::{RenderOptions, render_image},
};

use super::BenchmarkProfile;

const EXPORT_WIDTH: u32 = 6000;
const EXPORT_HEIGHT: u32 = 4000;
const EDIT_SAMPLES: usize = 12;

pub(super) fn benchmark(strict: bool, profile: BenchmarkProfile) -> Result<Value> {
    let directory =
        std::env::temp_dir().join(format!("spectrum-image-bench-{}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory)?;
    }
    fs::create_dir_all(&directory)?;
    let result = measure(&directory, profile);
    let _ = fs::remove_dir_all(&directory);
    let report = result?;
    if strict && report["pass"] != json!(true) {
        bail!("performance budget missed: {report}");
    }
    Ok(report)
}

fn measure(directory: &std::path::Path, profile: BenchmarkProfile) -> Result<Value> {
    let source = directory.join("24mp-source.jpg");
    DynamicImage::ImageRgb8(deterministic_rgb(EXPORT_WIDTH, EXPORT_HEIGHT))
        .save(&source)
        .context("prepare deterministic 24 MP benchmark source")?;
    let actor = Actor {
        id: "benchmark:image".into(),
        display_name: "Image benchmark".into(),
        kind: ActorKind::System,
    };
    let session = SessionId::new();
    let path = directory.join("image.spectrum");
    Workspace::create(&path, Image::import(&source)?, actor.clone(), session)?;
    let mut edit_samples = Vec::with_capacity(EDIT_SAMPLES);
    for iteration in 0..EDIT_SAMPLES {
        let started = Instant::now();
        let mut workspace = Workspace::open_newest(&path, actor.clone(), session)?;
        let mut adjustments = workspace.document.adjustments.clone();
        adjustments.curves.master = curve(if iteration % 2 == 0 { 0.42 } else { 0.58 });
        workspace.execute(Command::SetAdjustments { adjustments })?;
        edit_samples.push(started.elapsed());
    }

    let adjustments = Adjustments {
        exposure: 0.35,
        contrast: 12.0,
        shadows: 18.0,
        vibrance: 8.0,
        curves: ToneCurves {
            master: curve(0.35),
            ..Default::default()
        },
        ..Default::default()
    };
    let preview = render_samples(1800, 1200, 8, &adjustments);
    let live_preview = render_samples(960, 640, 12, &adjustments);

    let image = Workspace::read(&path)?;
    let export_path = directory.join("24mp-export.jpg");
    let started = Instant::now();
    spectrum_image::engine::export(&image, &export_path, RenderOptions::default(), 90)?;
    let export_ms = duration_ms(started.elapsed());

    let metrics = vec![
        latency_metric(
            "tone_curve_edit",
            "Reopen a 24 MP image document, apply a tone curve, and save one revision",
            &edit_samples,
            50.0,
            profile.edit_budget_ms(),
        ),
        latency_metric(
            "tone_curve_preview",
            "1800x1200 developed preview frame",
            &preview,
            16.7,
            profile.preview_budget_ms(),
        ),
        latency_metric(
            "adjustment_live_preview",
            "960x640 bounded interaction frame",
            &live_preview,
            16.7,
            profile.live_preview_budget_ms(),
        ),
        json!({
            "name": "full_export",
            "workload": "24 MP JPEG export",
            "elapsed_ms": rounded(export_ms),
            "target_ms": 2000.0,
            "budget_ms": 5000.0,
            "pass": export_ms <= 5000.0,
        }),
    ];
    let pass = metrics.iter().all(|metric| metric["pass"] == json!(true));
    Ok(json!({
        "ok": true,
        "profile": profile.name(),
        "pass": pass,
        "budgets": "Targets describe excellent feel; budgets are regression limits.",
        "metrics": metrics,
    }))
}

fn render_samples(
    width: u32,
    height: u32,
    count: usize,
    adjustments: &Adjustments,
) -> Vec<Duration> {
    let source = DynamicImage::ImageRgb8(deterministic_rgb(width, height));
    std::hint::black_box(render_image(
        source.clone(),
        adjustments.clone(),
        RenderOptions::default(),
    ));
    (0..count)
        .map(|_| {
            let started = Instant::now();
            std::hint::black_box(render_image(
                source.clone(),
                adjustments.clone(),
                RenderOptions::default(),
            ));
            started.elapsed()
        })
        .collect()
}

fn curve(middle: f32) -> ToneCurve {
    ToneCurve {
        points: vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.5, y: middle },
            CurvePoint { x: 1.0, y: 1.0 },
        ],
    }
}

fn deterministic_rgb(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| {
        Rgb([
            ((x * 13 + y * 3) % 256) as u8,
            ((x * 5 + y * 11) % 256) as u8,
            ((x * 7 + y * 17) % 256) as u8,
        ])
    })
}

fn latency_metric(
    name: &str,
    workload: &str,
    samples: &[Duration],
    target_ms: f64,
    budget_ms: f64,
) -> Value {
    let mut milliseconds: Vec<_> = samples.iter().copied().map(duration_ms).collect();
    milliseconds.sort_by(f64::total_cmp);
    let p95 = percentile(&milliseconds, 0.95);
    json!({
        "name": name,
        "workload": workload,
        "samples": milliseconds.len(),
        "median_ms": rounded(percentile(&milliseconds, 0.5)),
        "p95_ms": rounded(p95),
        "target_ms": target_ms,
        "budget_ms": budget_ms,
        "pass": p95 <= budget_ms,
    })
}

fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    let index = ((sorted.len().saturating_sub(1)) as f64 * quantile).ceil() as usize;
    sorted[index]
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn rounded(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
