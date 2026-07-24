use std::{io::Write, path::PathBuf, time::Instant};

use anyhow::{Result, bail};
use clap::ValueEnum;
use prism_core::{
    BlendMode, Command, Document, DropShadow, FontAsset, GradientStop, Layer, LayerKind, LayerMask,
    LayerStyle, RenderRegion, ShapeFill, ShapeGradient, ShapeStroke, TextAlignment, TextEffects,
    TextTypography, Transform, Workspace, region_source_scales, render_document,
    render_document_region_scaled, render_document_region_scaled_with_sources_and_stats,
    render_document_region_scaled_with_stats, render_layer_base_scaled,
    render_layer_base_scaled_with_font, render_solid_color,
};
use serde::Serialize;
use serde_json::{Value, json};
use spectrum_imaging::Adjustments;

#[path = "benchmark/raster_fixture.rs"]
mod raster_fixture;
use raster_fixture::PreparedRasterFixture;
#[path = "benchmark/temporary_font.rs"]
mod temporary_font;
use temporary_font::TemporaryFont;
#[path = "benchmark/adjusted_vector.rs"]
mod adjusted_vector;
#[path = "benchmark/dissolve_preview.rs"]
mod dissolve_preview;
#[path = "benchmark/font_picker.rs"]
mod font_picker;
#[path = "benchmark/paint.rs"]
mod paint;
#[path = "benchmark/path.rs"]
mod path;
#[path = "benchmark/selection.rs"]
mod selection;
#[path = "benchmark/selection_outline.rs"]
mod selection_outline;
#[path = "benchmark/text_preview_frame.rs"]
mod text_preview_frame;

#[derive(Clone, Copy, Default, ValueEnum)]
pub(super) enum BenchmarkProfile {
    #[default]
    Interactive,
    HostedCi,
}

impl BenchmarkProfile {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Interactive => "interactive-workstation",
            Self::HostedCi => "github-hosted-linux",
        }
    }

    pub(super) fn gradient_shadow_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 500.0,
            // The reviewed implementation measured 880.788 ms p95 on GitHub's
            // shared Linux runner versus 222.508 ms locally. A 1,250 ms ceiling
            // keeps 42% host-jitter headroom while the original 2,061.886 ms
            // regression still fails decisively.
            Self::HostedCi => 1_250.0,
        }
    }

    pub(super) fn magic_wand_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 5_000.0,
            Self::HostedCi => 15_000.0,
        }
    }

    pub(super) fn path_raster_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 250.0,
            Self::HostedCi => 750.0,
        }
    }

    pub(super) fn path_edit_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 5.0,
            Self::HostedCi => 15.0,
        }
    }

    pub(super) fn paint_viewport_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 500.0,
            Self::HostedCi => 1_500.0,
        }
    }
}

#[derive(Serialize)]
struct BenchmarkMetric {
    name: &'static str,
    median_ms: f64,
    p95_ms: f64,
    budget_ms: f64,
    pass: bool,
}

// spectrum-imaging expands adjusted regions by four source pixels for denoise
// and two more for sharpening.
const DEVELOPMENT_FILTER_HALO: f64 = 6.0;

fn bounded_staging_budget(
    document: &Document,
    document_scale: f32,
    region: RenderRegion,
) -> Result<u64> {
    document
        .layers
        .iter()
        .filter(|layer| layer.visible && layer.opacity > 0.0)
        .map(|layer| layer_staging_budget(document, layer, document_scale, region))
        .try_fold(0, |maximum, budget| -> Result<u64> {
            Ok(maximum.max(budget?))
        })
}

fn layer_staging_budget(
    document: &Document,
    layer: &Layer,
    document_scale: f32,
    region: RenderRegion,
) -> Result<u64> {
    let scales = region_source_scales(document, layer, document_scale)?;
    let (rotated_width, rotated_height) = inverse_rotation_aabb(
        f64::from(region.width),
        f64::from(region.height),
        layer.transform.rotation,
    );
    let adjusted_width = triangle_source_extent(rotated_width, scales.outer_transform[0]);
    let adjusted_height = triangle_source_extent(rotated_height, scales.outer_transform[1]);
    let adjusted_pixels = adjusted_width.saturating_mul(adjusted_height);

    // The development pipeline expands in adjusted coordinates before inverse
    // straighten sampling. Ignoring straighten's compensating zoom keeps this
    // an upper bound, and the extra two pixels cover its bilinear support.
    let halo = DEVELOPMENT_FILTER_HALO * 2.0;
    let (source_width, source_height) = inverse_rotation_aabb(
        adjusted_width as f64 + halo,
        adjusted_height as f64 + halo,
        layer.adjustments.straighten,
    );
    let bilinear_support = u64::from(layer.adjustments.straighten.abs() > 0.01) * 2;
    let source_width = source_width.ceil() as u64 + bilinear_support;
    let source_height = source_height.ceil() as u64 + bilinear_support;
    let source_pixels = source_width.saturating_mul(source_height);

    // Quarter-turn development rotations only swap the axes, so they do not
    // change the pixel-area bound.
    Ok(adjusted_pixels.max(source_pixels))
}

fn inverse_rotation_aabb(width: f64, height: f64, degrees: f32) -> (f64, f64) {
    let radians = f64::from(degrees).to_radians();
    let (sin, cos) = radians.sin_cos();
    (
        width * cos.abs() + height * sin.abs(),
        width * sin.abs() + height * cos.abs(),
    )
}

fn triangle_source_extent(output_extent: f64, outer_scale: f32) -> u64 {
    let inverse_scale = 1.0 / f64::from(outer_scale.abs().max(f32::EPSILON));
    let filter_radius = inverse_scale.max(1.0);
    (output_extent * inverse_scale + filter_radius * 2.0 + 2.0).ceil() as u64
}

pub(super) fn benchmark(strict: bool, profile: BenchmarkProfile) -> Result<Value> {
    let selection_fill = selection::measure_selection_fill()?;
    let magic_wand = selection::measure_magic_wand_bound()?;
    let lasso = selection::measure_lasso_bound()?;
    let selection_outline = selection_outline::measure()?;
    let path = path::measure()?;
    let paint = paint::measure()?;
    let text_preview_frame = text_preview_frame::measure()?;
    let font_picker = font_picker::measure();
    let dissolve_preview = dissolve_preview::measure()?;
    let dissolve_preview_budget =
        dissolve_preview::budget_ms(matches!(profile, BenchmarkProfile::HostedCi));
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
    let typography_font = TemporaryFont::new()?;
    let font_asset = FontAsset::import(1, &typography_font.path)?;
    let typography_layer = Layer {
        id: 88,
        kind: LayerKind::Text {
            text: "Portable typography wraps with measured rhythm and bounded effects".into(),
            font_size: 72.0,
            color: [245, 242, 235, 255],
            typography: TextTypography {
                font_id: Some(font_asset.id),
                alignment: TextAlignment::Center,
                line_height: 1.35,
                tracking: 2.0,
                box_width: Some(720.0),
                effects: TextEffects {
                    outline_width: 2.0,
                    shadow_offset_x: 4.0,
                    shadow_offset_y: 6.0,
                    shadow_color: [0, 0, 0, 128],
                    ..Default::default()
                },
            },
        },
        ..Layer::default()
    };
    let _ =
        render_layer_base_scaled_with_font(&typography_layer, None, [1.0; 2], Some(&font_asset))?;
    let mut typography_samples = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        let _ = render_layer_base_scaled_with_font(
            &typography_layer,
            None,
            [1.0; 2],
            Some(&font_asset),
        )?;
        typography_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
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
    let mut adjusted_bounded_sources = bounded_sources.clone();
    adjusted_bounded_sources.layers[0].adjustments = Adjustments {
        exposure: 0.22,
        vignette: -14.0,
        noise_reduction: 10.0,
        sharpening: 8.0,
        rotation: 90,
        straighten: 3.0,
        ..Default::default()
    };
    adjusted_bounded_sources.layers[1].adjustments = Adjustments {
        contrast: 11.0,
        vignette: -9.0,
        rotation: 270,
        straighten: -2.0,
        ..Default::default()
    };
    let bounded_region = RenderRegion {
        x: 8_000,
        y: 400,
        width: 960,
        height: 540,
    };
    let bounded_source_staging_budget =
        bounded_staging_budget(&bounded_sources, 1.0, bounded_region)?;
    let adjusted_bounded_staging_budget =
        bounded_staging_budget(&adjusted_bounded_sources, 1.0, bounded_region)?;
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
            || stats.max_source_staging_pixels > bounded_source_staging_budget
            || stats.max_adjusted_staging_pixels > bounded_source_staging_budget
            || stats.fallback_decode_bytes != 0
            || stats.transformed_surface_pixels != 0
        {
            bail!("bounded source compositor regressed to full-source staging");
        }
        bounded_source_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let mut adjusted_bounded_source_samples = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        let (rendered, stats) = render_document_region_scaled_with_stats(
            &adjusted_bounded_sources,
            1.0,
            bounded_region,
        )?;
        if (rendered.width(), rendered.height()) != (bounded_region.width, bounded_region.height)
            || stats.full_source_pixels <= 4_096 * 4_096
            || stats.source_staging_pixels >= stats.full_source_pixels
            || stats.adjusted_staging_pixels == 0
            || stats.max_source_staging_pixels > adjusted_bounded_staging_budget
            || stats.max_adjusted_staging_pixels > adjusted_bounded_staging_budget
            || stats.fallback_decode_bytes != 0
            || stats.transformed_surface_pixels != 0
        {
            bail!("adjusted bounded source compositor regressed to full-source staging");
        }
        adjusted_bounded_source_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let mut vector_sources = Document::new("Vector viewport", 16_384, 16_384);
    vector_sources.layers = vec![
        Layer {
            id: 110,
            opacity: 0.83,
            style: LayerStyle {
                drop_shadow: Some(DropShadow {
                    color: [0, 0, 0, 150],
                    offset_x: 18.0,
                    offset_y: 16.0,
                    blur_radius: 12.0,
                }),
            },
            shape_fill: Some(ShapeFill::Gradient(ShapeGradient {
                angle: 28.0,
                stops: vec![
                    GradientStop::new(0.0, [64, 142, 220, 218]),
                    GradientStop::new(1.0, [181, 92, 220, 218]),
                ],
                ..ShapeGradient::default()
            })),
            transform: Transform {
                rotation: 17.0,
                ..Transform::default()
            },
            stroke: ShapeStroke {
                enabled: true,
                width: 12.0,
                color: [244, 215, 128, 255],
            },
            kind: LayerKind::Ellipse {
                width: 16_384,
                height: 16_384,
                color: [64, 142, 220, 218],
            },
            ..Layer::default()
        },
        Layer {
            id: 111,
            opacity: 0.71,
            blend_mode: BlendMode::SoftLight,
            clip_to_below: true,
            transform: Transform {
                x: 1_500.0,
                y: 1_200.0,
                rotation: -11.0,
                ..Transform::default()
            },
            mask: LayerMask {
                enabled: true,
                invert: true,
                x: 0.2,
                y: 0.2,
                width: 0.6,
                height: 0.6,
            },
            stroke: ShapeStroke {
                enabled: true,
                width: 10.0,
                color: [87, 220, 231, 224],
            },
            kind: LayerKind::Rectangle {
                width: 12_000,
                height: 10_000,
                color: [220, 82, 154, 190],
                corner_radius: 180.0,
            },
            ..Layer::default()
        },
    ];
    let vector_region = RenderRegion {
        x: 72_000,
        y: 72_000,
        width: 640,
        height: 360,
    };
    let vector_staging_budget = bounded_staging_budget(&vector_sources, 8.0, vector_region)?;
    let mut vector_source_samples = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let (rendered, stats) =
            render_document_region_scaled_with_stats(&vector_sources, 8.0, vector_region)?;
        if (rendered.width(), rendered.height()) != (vector_region.width, vector_region.height)
            || stats.source_staging_pixels != 0
            || stats.max_source_staging_pixels > vector_staging_budget
            || stats.max_adjusted_staging_pixels > vector_staging_budget
            || stats.transformed_surface_pixels != 0
            || stats.shadow_samples > stats.output_pixels * 13
            || stats.shadow_alpha_tile_pixels == 0
            || stats.shadow_alpha_tile_bytes != stats.shadow_alpha_tile_pixels
            || stats.max_shadow_alpha_tile_pixels > 4_096 * 4_096
            || stats.max_shadow_alpha_tile_bytes != stats.max_shadow_alpha_tile_pixels
        {
            bail!("vector viewport compositor violated its allocation contract");
        }
        vector_source_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let mut adjusted_vector = adjusted_vector::measure(&vector_sources, vector_region)?;
    // Cold TIFF decode/publication is setup work and intentionally excluded
    // from the warm viewport samples below.
    let (cached_raster_source, cached_raster_cold_prepare) =
        PreparedRasterFixture::prepare(2_048, 1_536)?;
    let mut cached_spot_document = Document::new("Cached spot viewport", 4_096, 4_096);
    cached_spot_document.layers.push(Layer {
        id: 112,
        opacity: 0.84,
        blend_mode: BlendMode::Overlay,
        transform: Transform {
            x: 500.0,
            y: 400.0,
            rotation: 7.0,
            ..Transform::default()
        },
        mask: LayerMask {
            enabled: true,
            invert: true,
            x: 0.08,
            y: 0.1,
            width: 0.84,
            height: 0.8,
        },
        adjustments: Adjustments {
            exposure: 0.18,
            sharpening: 4.0,
            spots: vec![spectrum_imaging::SpotRemoval {
                x: 0.5,
                y: 0.5,
                radius: 0.012,
                opacity: 0.85,
            }],
            ..Default::default()
        },
        kind: LayerKind::Raster {
            path: cached_raster_source.source_path().to_owned(),
            original_path: None,
        },
        ..Layer::default()
    });
    let cached_spot_region = RenderRegion {
        x: 11_800,
        y: 9_000,
        width: 640,
        height: 360,
    };
    let mut cached_spot_samples = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let (rendered, stats) = render_document_region_scaled_with_sources_and_stats(
            &cached_spot_document,
            8.0,
            cached_spot_region,
            &cached_raster_source,
        )?;
        if (rendered.width(), rendered.height())
            != (cached_spot_region.width, cached_spot_region.height)
            || stats.fallback_decode_bytes != 0
            || stats.transformed_surface_pixels != 0
            || stats.max_source_staging_pixels >= 2_048 * 1_536
        {
            bail!("cached spot viewport compositor regressed to full-source work");
        }
        cached_spot_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let (command_median, command_p95) = sample_summary(&mut command_samples);
    let (interaction_median, interaction_p95) = sample_summary(&mut interaction_samples);
    let (text_interaction_median, text_interaction_p95) =
        sample_summary(&mut text_interaction_samples);
    let (shape_median, shape_p95) = sample_summary(&mut shape_preview_samples);
    let (render_median, render_p95) = sample_summary(&mut render_samples);
    let (scaled_shape_median, scaled_shape_p95) = sample_summary(&mut scaled_shape_samples);
    let (typography_median, typography_p95) = sample_summary(&mut typography_samples);
    let (blend_render_median, blend_render_p95) = sample_summary(&mut blend_render_samples);
    let (viewport_composite_median, viewport_composite_p95) =
        sample_summary(&mut viewport_composite_samples);
    let (bounded_source_median, bounded_source_p95) = sample_summary(&mut bounded_source_samples);
    let (vector_source_median, vector_source_p95) = sample_summary(&mut vector_source_samples);
    let (adjusted_bounded_source_median, adjusted_bounded_source_p95) =
        sample_summary(&mut adjusted_bounded_source_samples);
    let (adjusted_vector_source_median, adjusted_vector_source_p95) =
        sample_summary(&mut adjusted_vector.samples);
    let (cached_spot_median, cached_spot_p95) = sample_summary(&mut cached_spot_samples);
    let gradient_shadow_budget_ms = profile.gradient_shadow_budget_ms();
    let metrics = [
        BenchmarkMetric {
            name: "4096_square_contiguous_magic_wand",
            median_ms: magic_wand.elapsed_ms,
            p95_ms: magic_wand.elapsed_ms,
            budget_ms: profile.magic_wand_budget_ms(),
            pass: magic_wand.elapsed_ms <= profile.magic_wand_budget_ms()
                && magic_wand.major_plane_bytes <= 112 * 1024 * 1024
                && magic_wand.logical_peak_bytes <= 120 * 1024 * 1024,
        },
        BenchmarkMetric {
            name: "nondestructive_selection_fill_command",
            median_ms: selection_fill.median_ms,
            p95_ms: selection_fill.p95_ms,
            budget_ms: 5.0,
            pass: selection_fill.p95_ms <= 5.0,
        },
        BenchmarkMetric {
            name: "16384_canvas_bounded_lasso_selection",
            median_ms: lasso.elapsed_ms,
            p95_ms: lasso.elapsed_ms,
            budget_ms: 500.0,
            pass: lasso.elapsed_ms <= 500.0 && lasso.mask_pixels <= 192 * 192,
        },
        BenchmarkMetric {
            name: "pathological_selection_contour_and_animated_frame",
            median_ms: selection_outline.median_ms,
            p95_ms: selection_outline.p95_ms,
            budget_ms: 100.0,
            pass: selection_outline.p95_ms <= 100.0,
        },
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
            name: "seeded_dissolve_direct_transform_composite",
            median_ms: dissolve_preview.median_ms,
            p95_ms: dissolve_preview.p95_ms,
            budget_ms: dissolve_preview_budget,
            pass: dissolve_preview.p95_ms <= dissolve_preview_budget,
        },
        BenchmarkMetric {
            name: "gui_long_text_cold_face_cached_preview_frame",
            median_ms: text_preview_frame.scheduling_median_ms,
            p95_ms: text_preview_frame.scheduling_p95_ms,
            budget_ms: 0.05,
            pass: text_preview_frame.scheduling_p95_ms <= 0.05,
        },
        BenchmarkMetric {
            name: "10k_font_search_and_reversible_hover_dispatch",
            median_ms: font_picker.median_ms,
            p95_ms: font_picker.p95_ms,
            budget_ms: 50.0,
            pass: font_picker.p95_ms <= 50.0,
        },
        BenchmarkMetric {
            name: "cold_imported_font_identity",
            median_ms: text_preview_frame.cold_import_ms,
            p95_ms: text_preview_frame.cold_import_ms,
            budget_ms: 50.0,
            pass: text_preview_frame.cold_import_ms <= 50.0,
        },
        BenchmarkMetric {
            name: "cold_imported_text_edit_geometry",
            median_ms: text_preview_frame.cold_edit_median_ms,
            p95_ms: text_preview_frame.cold_edit_p95_ms,
            budget_ms: 100.0,
            pass: text_preview_frame.cold_edit_p95_ms <= 100.0,
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
            name: "16x_256_anchor_cubic_path_raster",
            median_ms: path.raster_median_ms,
            p95_ms: path.raster_p95_ms,
            budget_ms: profile.path_raster_budget_ms(),
            pass: path.raster_p95_ms <= profile.path_raster_budget_ms(),
        },
        BenchmarkMetric {
            name: "256_anchor_path_edit_preview",
            median_ms: path.edit_median_ms,
            p95_ms: path.edit_p95_ms,
            budget_ms: profile.path_edit_budget_ms(),
            pass: path.edit_p95_ms <= profile.path_edit_budget_ms(),
        },
        BenchmarkMetric {
            name: "16k_max_sample_sparse_paint_viewport",
            median_ms: paint.median_ms,
            p95_ms: paint.p95_ms,
            budget_ms: profile.paint_viewport_budget_ms(),
            pass: paint.p95_ms <= profile.paint_viewport_budget_ms(),
        },
        BenchmarkMetric {
            name: "portable_typography_effect_raster",
            median_ms: typography_median,
            p95_ms: typography_p95,
            budget_ms: 75.0,
            pass: typography_p95 <= 75.0,
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
        BenchmarkMetric {
            name: "8x_zoom_16k_gradient_shadow_viewport_composite",
            median_ms: vector_source_median,
            p95_ms: vector_source_p95,
            budget_ms: gradient_shadow_budget_ms,
            pass: vector_source_p95 <= gradient_shadow_budget_ms,
        },
        BenchmarkMetric {
            name: "large_adjusted_raster_text_bounded_staging",
            median_ms: adjusted_bounded_source_median,
            p95_ms: adjusted_bounded_source_p95,
            budget_ms: 1_000.0,
            pass: adjusted_bounded_source_p95 <= 1_000.0,
        },
        BenchmarkMetric {
            name: "8x_zoom_16k_adjusted_vector_viewport_composite",
            median_ms: adjusted_vector_source_median,
            p95_ms: adjusted_vector_source_p95,
            budget_ms: 750.0,
            pass: adjusted_vector_source_p95 <= 750.0,
        },
        BenchmarkMetric {
            name: "8x_zoom_cached_spot_raster_viewport_composite",
            median_ms: cached_spot_median,
            p95_ms: cached_spot_p95,
            budget_ms: 500.0,
            pass: cached_spot_p95 <= 500.0,
        },
    ];
    let passed = metrics.iter().all(|metric| metric.pass);
    if strict && !passed {
        let failures = metrics
            .iter()
            .filter(|metric| !metric.pass)
            .map(|metric| {
                format!(
                    "{} p95 {:.3} ms > {:.3} ms",
                    metric.name, metric.p95_ms, metric.budget_ms
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        bail!("Prism benchmark exceeded a strict regression budget: {failures}");
    }
    Ok(json!({
        "ok": true,
        "action": "benchmark",
        "strict": strict,
        "profile": profile.name(),
        "passed": passed,
        "output": [rendered.as_ref().unwrap().width(), rendered.as_ref().unwrap().height()],
        "setup": {
            "cached_raster_cold_prepare_ms": cached_raster_cold_prepare.as_secs_f64() * 1_000.0,
            "cached_raster_samples": "warm file-backed provider reads",
            "paint_max_source_staging_pixels": paint.max_source_staging_pixels,
            "adjusted_vector_max_shadow_alpha_tile_pixels": adjusted_vector.max_shadow_alpha_tile_pixels
        },
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

#[cfg(test)]
mod staging_bound_tests {
    use super::*;

    #[test]
    fn inverse_rotation_uses_the_source_space_aabb() {
        let (width, height) = inverse_rotation_aabb(960.0, 540.0, 90.0);
        assert!((width - 540.0).abs() < 0.001);
        assert!((height - 960.0).abs() < 0.001);

        let (width, height) = inverse_rotation_aabb(960.0, 540.0, 45.0);
        assert!(width > 960.0);
        assert!(height > 960.0);
    }

    #[test]
    fn triangle_support_tracks_upscale_and_downscale() {
        assert_eq!(triangle_source_extent(100.0, 1.0), 104);
        assert_eq!(triangle_source_extent(100.0, 2.0), 54);
        assert_eq!(triangle_source_extent(100.0, 0.5), 206);
    }
}
