//! What the canvas does on every frame of a drag: rotating a large image,
//! dragging a style slider on it, erasing it or text, drawing a brush
//! stroke, rotating text, and sizing text in an imported font.
//! Each must stay inside a frame budget, the way the preview renders them:
//! large layers as drafts within a pixel budget, images from the
//! interactive source cache, and strokes as a patch under the brush.
use std::{hint::black_box, time::Instant};

use anyhow::Result;
use spectrum_canvas::{
    BrushMode, BrushSample, BrushStroke, BrushStyle, Command, Document, GradientOverlay, Layer,
    LayerKind, PaintSelection, Transform, Workspace, prepare_export_raster_sources,
    render_document_scaled_with_sources, render_paint_layer_region,
};

use super::{BenchmarkMetric, sample_summary};

/// The preview's draft budget, in device pixels.
const DRAFT_PIXELS: f32 = 1_200_000.;

fn frame_budget(hosted: bool, local_ms: f64) -> f64 {
    // Shared CI runners get 2.5x for scheduling noise; the local budget is
    // the product requirement.
    if hosted { local_ms * 2.5 } else { local_ms }
}

fn metric(name: &'static str, samples: &mut [f64], budget_ms: f64) -> BenchmarkMetric {
    let (median_ms, p95_ms) = sample_summary(samples);
    BenchmarkMetric {
        name,
        median_ms,
        p95_ms,
        budget_ms,
        pass: p95_ms <= budget_ms,
    }
}

fn photo_document(directory: &std::path::Path) -> Result<Document> {
    let path = directory.join("photo.png");
    image::RgbaImage::from_fn(2560, 1600, |x, y| {
        image::Rgba([(x % 251) as u8, (y % 241) as u8, ((x ^ y) % 256) as u8, 255])
    })
    .save(&path)?;
    let mut document = Document::new("Interactive photo", 1920, 1080);
    document.layers.push(Layer {
        id: 1,
        kind: LayerKind::Raster { path },
        transform: Transform {
            x: 180.0,
            y: 60.0,
            scale_x: 0.6,
            scale_y: 0.6,
            rotation: 0.0,
        },
        ..Layer::default()
    });
    document.next_id = 2;
    Ok(document)
}

/// A draft density for a layer covering `area` canvas pixels at retina 2x.
fn draft_density(area: f32) -> f32 {
    let density = 2.0;
    density
        * (DRAFT_PIXELS / (area * density * density))
            .sqrt()
            .clamp(0.25, 1.0)
}

pub(super) fn metrics(hosted: bool) -> Result<Vec<BenchmarkMetric>> {
    spectrum_canvas::set_interactive_source_cache(true);
    let result = measure(hosted);
    spectrum_canvas::set_interactive_source_cache(false);
    result
}

fn measure(hosted: bool) -> Result<Vec<BenchmarkMetric>> {
    let directory = std::env::temp_dir().join(format!(
        "spectrum-interactive-benchmark-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory)?;
    let cache = directory.join("cache");
    let mut document = photo_document(&directory)?;
    let sources = prepare_export_raster_sources(&document, &cache)?;
    let density = draft_density(1536.0 * 960.0);
    let frames = 24;
    // The layer is on screen at full sharpness before a drag starts.
    black_box(render_document_scaled_with_sources(
        &document, 2.0, &sources,
    )?);

    // Rotating the photo by its handle.
    let mut rotate = Vec::with_capacity(frames);
    for frame in 0..frames {
        document.layers[0].transform.rotation = frame as f32 * 3.0;
        let started = Instant::now();
        black_box(render_document_scaled_with_sources(
            &document, density, &sources,
        )?);
        rotate.push(started.elapsed().as_secs_f64() * 1_000.0);
    }

    // Dragging a gradient overlay's angle on it.
    document.layers[0].transform.rotation = 0.0;
    let mut overlay = Vec::with_capacity(frames);
    for frame in 0..frames {
        let mut style = GradientOverlay::default();
        style.gradient.angle = frame as f32 * 7.0;
        document.layers[0].style.gradient_overlay = Some(style);
        let started = Instant::now();
        black_box(render_document_scaled_with_sources(
            &document, density, &sources,
        )?);
        overlay.push(started.elapsed().as_secs_f64() * 1_000.0);
    }

    // Erasing across the photo: the stroke so far taken out of its mask.
    document.layers[0].style.gradient_overlay = None;
    let erase_style = BrushStyle {
        mode: BrushMode::Erase,
        size: 40.0,
        hardness: 0.8,
        ..BrushStyle::default()
    };
    let path: Vec<BrushSample> = (0..48)
        .map(|i| BrushSample {
            x: 300.0 + i as f32 * 20.0,
            y: 400.0 + (i as f32 * 0.3).sin() * 120.0,
            pressure: 1.0,
        })
        .collect();
    let mut erase_image = Vec::with_capacity(frames);
    for end in (2..path.len()).step_by(2) {
        let started = Instant::now();
        let mut local = Workspace::new(document.clone());
        local.execute(Command::EraseLayer {
            id: 1,
            stroke: BrushStroke::new(erase_style, path[..end].to_vec())?,
        })?;
        black_box(render_document_scaled_with_sources(
            &local.document,
            density,
            &sources,
        )?);
        erase_image.push(started.elapsed().as_secs_f64() * 1_000.0);
    }

    // Drawing a stroke over a Paint layer that already has thirty.
    let mut workspace = Workspace::new(Document::new("Interactive paint", 1920, 1080));
    let style = BrushStyle {
        size: 36.0,
        hardness: 0.7,
        ..BrushStyle::default()
    };
    let line = |row: usize, count: usize| -> Vec<BrushSample> {
        (0..count)
            .map(|i| BrushSample {
                x: 40.0 + i as f32 * 9.0,
                y: 40.0 + row as f32 * 32.0 + (i as f32 * 0.4).sin() * 12.0,
                pressure: 1.0,
            })
            .collect()
    };
    workspace.execute(Command::AddPaintLayerWithStroke {
        name: None,
        width: 1920,
        height: 1080,
        stroke: BrushStroke::new(style, line(0, 200))?,
        selection: PaintSelection::None,
    })?;
    let id = workspace.document.layers[0].id;
    for row in 1..30 {
        workspace.execute(Command::AddBrushStroke {
            id,
            stroke: BrushStroke::new(style, line(row, 200))?,
            selection: PaintSelection::None,
        })?;
    }
    let mut stroke = Vec::with_capacity(60);
    let drawn: Vec<BrushSample> = (0..120)
        .map(|i| BrushSample {
            x: 300.0 + i as f32 * 8.0,
            y: 500.0 + (i as f32 * 0.2).cos() * 80.0,
            pressure: 1.0,
        })
        .collect();
    for end in (2..drawn.len()).step_by(2) {
        let started = Instant::now();
        let mut local = Workspace::new(workspace.document.clone());
        local.execute(Command::AddBrushStroke {
            id,
            stroke: BrushStroke::new(style, drawn[..end].to_vec())?,
            selection: PaintSelection::Current,
        })?;
        let (x0, y0, x1, y1) = drawn[..end].iter().fold(
            (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
            |(a, b, c, d), s| (a.min(s.x), b.min(s.y), c.max(s.x), d.max(s.y)),
        );
        let reach = style.size / 2.0 + 2.0;
        let region = (
            (x0 - reach).max(0.0) as u32,
            (y0 - reach).max(0.0) as u32,
            (x1 - x0 + 2.0 * reach) as u32,
            (y1 - y0 + 2.0 * reach) as u32,
        );
        let patch = render_paint_layer_region(&local.document, id, region)?;
        // The preview uploads it as BGRA.
        let mut bgra = patch.into_raw();
        for pixel in bgra.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        black_box(bgra);
        stroke.push(started.elapsed().as_secs_f64() * 1_000.0);
    }

    // Rotating large text at full retina sharpness.
    let mut text_document = Document::new("Interactive text", 1920, 1080);
    text_document.layers.push(Layer {
        id: 1,
        kind: LayerKind::Text {
            text: "Spring sale!".into(),
            font_size: 160.0,
            color: [255, 255, 255, 255],
            typography: Default::default(),
        },
        transform: Transform {
            x: 300.0,
            y: 300.0,
            ..Transform::default()
        },
        ..Layer::default()
    });
    text_document.next_id = 2;
    let mut text = Vec::with_capacity(frames);
    for frame in 0..frames {
        text_document.layers[0].transform.rotation = frame as f32 * 4.0;
        let geometry =
            spectrum_canvas::document_layer_geometry(&text_document, &text_document.layers[0])?;
        let region = spectrum_canvas::RenderRegion {
            x: (geometry.min[0] * 2.0).max(0.0) as u32,
            y: (geometry.min[1] * 2.0).max(0.0) as u32,
            width: (geometry.width() * 2.0).ceil() as u32,
            height: (geometry.height() * 2.0).ceil() as u32,
        };
        let started = Instant::now();
        black_box(spectrum_canvas::render_document_region_scaled(
            &text_document,
            2.0,
            region,
        )?);
        text.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    // Erasing across the text, at full sharpness.
    text_document.layers[0].transform.rotation = 0.0;
    let geometry =
        spectrum_canvas::document_layer_geometry(&text_document, &text_document.layers[0])?;
    let region = spectrum_canvas::RenderRegion {
        x: (geometry.min[0] * 2.0).max(0.0) as u32,
        y: (geometry.min[1] * 2.0).max(0.0) as u32,
        width: (geometry.width() * 2.0).ceil() as u32,
        height: (geometry.height() * 2.0).ceil() as u32,
    };
    let across: Vec<BrushSample> = (0..48)
        .map(|i| BrushSample {
            x: geometry.min[0] + i as f32 * geometry.width() / 47.0,
            y: geometry.center[1] + (i as f32 * 0.4).sin() * 40.0,
            pressure: 1.0,
        })
        .collect();
    let mut erase_text = Vec::with_capacity(frames);
    for end in (2..across.len()).step_by(2) {
        let started = Instant::now();
        let mut local = Workspace::new(text_document.clone());
        local.execute(Command::EraseLayer {
            id: 1,
            stroke: BrushStroke::new(erase_style, across[..end].to_vec())?,
        })?;
        black_box(spectrum_canvas::render_document_region_scaled(
            &local.document,
            2.0,
            region,
        )?);
        erase_text.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    // Dragging the size of text in an imported font, a step a frame.
    let font_path = directory.join("imported.ttf");
    std::fs::write(&font_path, epaint_default_fonts::HACK_REGULAR)?;
    text_document.layers[0].transform.rotation = 0.0;
    let mut sized = Workspace::new(text_document.clone());
    sized.execute(Command::ImportFont {
        path: font_path,
        source_name: None,
    })?;
    let LayerKind::Text { typography, .. } = &sized.document.layers[0].kind else {
        anyhow::bail!("the benchmark text layer is not text");
    };
    let typography = spectrum_canvas::TextTypography {
        font_id: sized.document.font_assets.first().map(|font| font.id),
        ..typography.clone()
    };
    sized.execute(Command::SetTextTypography { id: 1, typography })?;
    let mut text_size = Vec::with_capacity(frames);
    for frame in 0..frames {
        let started = Instant::now();
        let mut local = Workspace::new(sized.document.clone());
        local.execute(Command::UpdateText {
            id: 1,
            text: "Spring sale!".into(),
            font_size: 160.0 + frame as f32 * 2.0,
            color: [255, 255, 255, 255],
        })?;
        let geometry =
            spectrum_canvas::document_layer_geometry(&local.document, &local.document.layers[0])?;
        let region = spectrum_canvas::RenderRegion {
            x: (geometry.min[0] * 2.0).max(0.0) as u32,
            y: (geometry.min[1] * 2.0).max(0.0) as u32,
            width: (geometry.width() * 2.0).ceil() as u32,
            height: (geometry.height() * 2.0).ceil() as u32,
        };
        black_box(spectrum_canvas::render_document_region_scaled(
            &local.document,
            2.0,
            region,
        )?);
        text_size.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    std::fs::remove_dir_all(&directory).ok();
    Ok(vec![
        metric(
            "interactive_rotate_2560px_image_draft_frame",
            &mut rotate,
            frame_budget(hosted, 33.0),
        ),
        metric(
            "interactive_gradient_overlay_on_image_draft_frame",
            &mut overlay,
            frame_budget(hosted, 33.0),
        ),
        metric(
            "interactive_erase_image_draft_frame",
            &mut erase_image,
            frame_budget(hosted, 33.0),
        ),
        metric(
            "interactive_erase_text_full_density_frame",
            &mut erase_text,
            frame_budget(hosted, 33.0),
        ),
        metric(
            "interactive_brush_stroke_patch_frame",
            &mut stroke,
            frame_budget(hosted, 16.0),
        ),
        metric(
            "interactive_text_size_imported_font_frame",
            &mut text_size,
            frame_budget(hosted, 16.0),
        ),
        metric(
            "interactive_rotate_text_full_density_frame",
            &mut text,
            frame_budget(hosted, 33.0),
        ),
    ])
}
