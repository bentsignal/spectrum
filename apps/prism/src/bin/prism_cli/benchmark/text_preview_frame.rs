use std::{collections::HashSet, hint::black_box, time::Instant};

use anyhow::{Result, bail};
use prism_core::{
    Document, FontAsset, Layer, LayerKind, LayerPreviewSchedule, TextGeometry,
    TextPreviewFrameCache, TextTypography, measure_text_geometry_with_typography,
};

use super::{TemporaryFont, sample_summary};

pub(super) struct Measurement {
    pub(super) scheduling_median_ms: f64,
    pub(super) scheduling_p95_ms: f64,
    pub(super) cold_import_ms: f64,
    pub(super) cold_edit_median_ms: f64,
    pub(super) cold_edit_p95_ms: f64,
}

pub(super) fn measure() -> Result<Measurement> {
    let fixture = TemporaryFont::new()?;
    let import_started = Instant::now();
    let font = FontAsset::import(911, &fixture.path)?;
    let cold_import_ms = import_started.elapsed().as_secs_f64() * 1_000.0;
    let cold_edit_text = "Cold imported text edit ".repeat(128);
    let cold_edit_typography = TextTypography {
        font_id: Some(font.id),
        ..TextTypography::default()
    };
    let mut cold_edit_samples = Vec::with_capacity(9);
    for _ in 0..9 {
        let started = Instant::now();
        black_box(measure_text_geometry_with_typography(
            &cold_edit_text,
            96.0,
            &cold_edit_typography,
            Some(&font),
        )?);
        cold_edit_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let (cold_edit_median_ms, cold_edit_p95_ms) = sample_summary(&mut cold_edit_samples);
    let mut document = Document::new("GUI scheduling benchmark", 1_600, 1_200);
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
    document.font_assets.push(font);
    document.layers.push(layer);
    let layer = document.layer(912)?;
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
    let visual_cache = HashSet::from([layer.id]);
    // Keep the imported face cold and make accidental font resolution fail
    // deterministically instead of relying on timing or process RSS.
    std::fs::remove_file(&fixture.path)?;

    let mut samples = Vec::with_capacity(240);
    for _ in 0..240 {
        let started = Instant::now();
        let scheduling =
            cache.schedule_layer(layer, true, visual_cache.contains(&layer.id), false, || {
                let cloned_kind = layer.kind.clone();
                let LayerKind::Text {
                    text,
                    font_size,
                    typography,
                    ..
                } = &cloned_kind
                else {
                    unreachable!("benchmark layer is text")
                };
                let font = document
                    .font_for_layer(layer)
                    .ok_or_else(|| anyhow::anyhow!("benchmark font identity was not resolved"))?;
                measure_text_geometry_with_typography(text, *font_size, typography, Some(font))
            });
        if !matches!(scheduling, LayerPreviewSchedule::ReuseCachedTextVisual) {
            bail!(
                "GUI text preview scheduling invoked key/font/shaping resolution instead of reusing caches"
            );
        }
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    black_box(document);
    let (scheduling_median_ms, scheduling_p95_ms) = sample_summary(&mut samples);
    Ok(Measurement {
        scheduling_median_ms,
        scheduling_p95_ms,
        cold_import_ms,
        cold_edit_median_ms,
        cold_edit_p95_ms,
    })
}
