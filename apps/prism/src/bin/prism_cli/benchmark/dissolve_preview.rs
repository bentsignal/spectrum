use std::{hint::black_box, time::Instant};

use anyhow::{Result, bail};
use eframe::egui;
use prism_core::{
    BlendMode, Document, Layer, LayerKind, RenderRegion, Transform,
    render_direct_preview_region_scaled,
};

use super::sample_summary;

pub(super) struct Measurement {
    pub(super) median_ms: f64,
    pub(super) p95_ms: f64,
}

pub(super) fn budget_ms(hosted_ci: bool) -> f64 {
    // The hosted profile runs the identical 2x render/conversion workload. Its
    // 2.5x ceiling only absorbs shared-runner CPU scheduling variance; the
    // 100 ms interactive gate remains the product-performance requirement.
    if hosted_ci { 250.0 } else { 100.0 }
}

pub(super) fn measure() -> Result<Measurement> {
    let mut document = Document::new("Direct Dissolve benchmark", 960, 540);
    document.background = [17, 23, 31, 255];
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            x: 180.0,
            y: 110.0,
            ..Transform::default()
        },
        opacity: 0.5,
        blend_mode: BlendMode::Dissolve,
        dissolve_seed: 0x1234_5678,
        kind: LayerKind::Ellipse {
            width: 480,
            height: 320,
            color: [230, 80, 150, 255],
        },
        ..Layer::default()
    });
    let region = RenderRegion {
        x: 0,
        y: 0,
        width: document.width * 2,
        height: document.height * 2,
    };
    let mut samples = Vec::with_capacity(15);
    for frame in 0..15 {
        document.layers[0].transform = Transform {
            x: 165.0 + frame as f32 * 2.0,
            y: 100.0 + frame as f32,
            scale_x: 1.0 + frame as f32 / 100.0,
            scale_y: 1.0 - frame as f32 / 200.0,
            rotation: frame as f32 * 2.0,
        };
        let started = Instant::now();
        let request_document = document.clone();
        let rendered = render_direct_preview_region_scaled(&request_document, 2.0, region)?;
        if (rendered.width(), rendered.height()) != (region.width, region.height) {
            bail!("direct Dissolve compositor returned the wrong physical dimensions");
        }
        let rgba = rendered.into_rgba8();
        let upload = egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        );
        black_box((request_document, upload));
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let (median_ms, p95_ms) = sample_summary(&mut samples);
    Ok(Measurement { median_ms, p95_ms })
}
