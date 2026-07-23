use std::{hint::black_box, time::Instant};

use anyhow::{Result, bail};
use prism_core::{
    FontAsset, Layer, LayerKind, TextGeometry, TextPreviewFrameCache, TextTypography,
    reuse_text_preview_frame,
};

use super::{TemporaryFont, sample_summary};

pub(super) struct Measurement {
    pub(super) median_ms: f64,
    pub(super) p95_ms: f64,
}

pub(super) fn measure() -> Result<Measurement> {
    let fixture = TemporaryFont::new()?;
    let font = FontAsset::import(911, &fixture.path)?;
    let layer = Layer {
        id: 912,
        kind: LayerKind::Text {
            text: "Cold imported preview geometry ".repeat(32_768),
            font_size: 96.0,
            color: [255; 4],
            typography: TextTypography {
                font_id: Some(font.id),
                ..TextTypography::default()
            },
        },
        ..Layer::default()
    };
    let mut cache = TextPreviewFrameCache::default();
    cache.insert(
        layer.id,
        TextGeometry {
            width: 2_048,
            height: 512,
            visual_left: 0.0,
            visual_top: 0.0,
            visual_width: 2_048.0,
            visual_height: 512.0,
            layout_left: 0.0,
            layout_top: 0.0,
            layout_width: 2_048.0,
            layout_height: 512.0,
        },
    );

    let mut samples = Vec::with_capacity(240);
    for _ in 0..240 {
        let started = Instant::now();
        let geometry_cached = cache.get(layer.id).is_some();
        let reused = reuse_text_preview_frame(
            true,
            matches!(layer.kind, LayerKind::Text { .. }),
            geometry_cached,
            true,
            false,
        );
        black_box(cache.get(layer.id));
        if !reused {
            bail!("GUI text preview frame unexpectedly left the geometry cache fast path");
        }
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    black_box(font);
    black_box(layer);
    let (median_ms, p95_ms) = sample_summary(&mut samples);
    Ok(Measurement { median_ms, p95_ms })
}
