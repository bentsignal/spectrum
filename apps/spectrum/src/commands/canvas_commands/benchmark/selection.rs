use std::time::Instant;

use anyhow::{Context, Result, bail};
use prism_core::{
    Command, Document, LassoPath, LassoPoint, LayerKind, Selection, SelectionCombineMode, Workspace,
};
use spectrum_imaging::AdjustmentPatch;

pub(super) struct SelectionFillSamples {
    pub(super) median_ms: f64,
    pub(super) p95_ms: f64,
}

pub(super) struct RasterDeleteSamples {
    pub(super) median_ms: f64,
    pub(super) p95_ms: f64,
    pub(super) near_cap_ms: f64,
}

pub(super) fn measure_color_mask_raster_delete() -> Result<RasterDeleteSamples> {
    const CANVAS_EDGE: u32 = 16_384;
    const SOURCE_EDGE: u32 = 164;
    let path = std::env::temp_dir().join(format!(
        "prism-raster-delete-benchmark-{}.png",
        std::process::id()
    ));
    image::RgbaImage::from_pixel(SOURCE_EDGE, SOURCE_EDGE, image::Rgba([80, 120, 220, 255]))
        .save(&path)?;
    let mut samples = Vec::with_capacity(17);
    for _ in 0..17 {
        let mut workspace = Workspace::new(
            Document::new("Raster delete bound", CANVAS_EDGE, CANVAS_EDGE),
            None,
        );
        workspace.execute(Command::AddRaster {
            path: path.clone(),
            name: None,
            x: 0.0,
            y: 0.0,
        })?;
        workspace.execute(Command::AdjustLayer {
            id: 1,
            patch: AdjustmentPatch {
                straighten: Some(3.0),
                ..Default::default()
            },
        })?;
        workspace.execute(Command::SetSelection {
            selection: Some(Selection::ColorMask {
                x: 0,
                y: 0,
                width: SOURCE_EDGE,
                height: SOURCE_EDGE,
                alpha: vec![255; (SOURCE_EDGE * SOURCE_EDGE) as usize].into(),
            }),
        })?;
        let started = Instant::now();
        workspace.execute(Command::DeleteSelectedPixels { id: 1 })?;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        let mask = workspace.document.layer(1)?.pixel_mask.as_ref().context(
            "ColorMask raster delete benchmark did not create a source-space pixel mask",
        )?;
        if mask.alpha.len() != (SOURCE_EDGE * SOURCE_EDGE) as usize
            || mask.alpha.iter().all(|alpha| *alpha != 0)
        {
            bail!("ColorMask raster delete benchmark produced the wrong source-space mask");
        }
    }
    let _ = std::fs::remove_file(path);
    samples.sort_by(f64::total_cmp);
    let near_cap_ms = measure_near_cap_color_mask_delete()?;
    Ok(RasterDeleteSamples {
        median_ms: samples[samples.len() / 2],
        p95_ms: samples[16],
        near_cap_ms,
    })
}

fn measure_near_cap_color_mask_delete() -> Result<f64> {
    const EDGE: u32 = 4_080;
    let path = std::env::temp_dir().join(format!(
        "prism-near-cap-raster-delete-benchmark-{}.png",
        std::process::id()
    ));
    image::RgbaImage::from_pixel(EDGE, EDGE, image::Rgba([80, 120, 220, 255])).save(&path)?;
    let mut workspace = Workspace::new(
        Document::new("Near-cap ColorMask raster delete", EDGE, EDGE),
        None,
    );
    workspace.execute(Command::AddRaster {
        path: path.clone(),
        name: None,
        x: 0.0,
        y: 0.0,
    })?;
    workspace.execute(Command::AdjustLayer {
        id: 1,
        patch: AdjustmentPatch {
            straighten: Some(1.0),
            ..Default::default()
        },
    })?;
    workspace.execute(Command::SetSelection {
        selection: Some(Selection::ColorMask {
            x: 0,
            y: 0,
            width: EDGE,
            height: EDGE,
            alpha: vec![255; (EDGE * EDGE) as usize].into(),
        }),
    })?;
    let started = Instant::now();
    workspace.execute(Command::DeleteSelectedPixels { id: 1 })?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let mask = workspace
        .document
        .layer(1)?
        .pixel_mask
        .as_ref()
        .context("near-cap ColorMask delete did not create a source-space mask")?;
    if mask.alpha.len() != (EDGE * EDGE) as usize || mask.alpha.iter().all(|alpha| *alpha != 0) {
        bail!("near-cap ColorMask delete produced the wrong bounded mask");
    }
    let _ = std::fs::remove_file(path);
    Ok(elapsed_ms)
}

pub(super) struct MagicWandSample {
    pub(super) elapsed_ms: f64,
    pub(super) major_plane_bytes: u64,
    pub(super) logical_peak_bytes: u64,
}

pub(super) struct LassoSample {
    pub(super) elapsed_ms: f64,
    pub(super) mask_pixels: u64,
}

pub(super) fn measure_lasso_bound() -> Result<LassoSample> {
    const EDGE: u32 = 16_384;
    let mut workspace = Workspace::new(Document::new("Lasso bound", EDGE, EDGE), None);
    let corners = [(8_000.0, 8_000.0), (8_192.0, 8_000.0), (8_000.0, 8_192.0)];
    let mut raw_points = Vec::with_capacity(8_190);
    for edge in 0..3 {
        let start = corners[edge];
        let end = corners[(edge + 1) % 3];
        for index in 0..2_730 {
            let t = index as f32 / 2_730.0;
            raw_points.push(LassoPoint::from_canvas(
                start.0 + (end.0 - start.0) * t,
                start.1 + (end.1 - start.1) * t,
            )?);
        }
    }
    let points = LassoPath::new(raw_points)?;
    let started = Instant::now();
    workspace.execute(Command::LassoSelection {
        points,
        mode: SelectionCombineMode::Replace,
        antialias: true,
    })?;
    let selection = workspace
        .document
        .selection
        .as_ref()
        .context("lasso benchmark did not create a selection")?;
    let bounds = selection.bounds();
    let mask_pixels = u64::from(bounds.2) * u64::from(bounds.3);
    if bounds != (8_000, 8_000, 192, 192)
        || selection.alpha().map(|alpha| alpha.len()) != Some(192 * 192)
    {
        bail!("large-canvas lasso benchmark did not stay inside its bounded alpha plane");
    }
    Ok(LassoSample {
        elapsed_ms: started.elapsed().as_secs_f64() * 1_000.0,
        mask_pixels,
    })
}

pub(super) fn measure_magic_wand_bound() -> Result<MagicWandSample> {
    const EDGE: u32 = 4_096;
    let mut workspace = Workspace::new(Document::new("Magic wand bound", EDGE, EDGE), None);
    let started = Instant::now();
    workspace.execute(Command::MagicWandSelection {
        x: EDGE / 2,
        y: EDGE / 2,
        tolerance: 0,
        contiguous: true,
        antialias: true,
        resolved_selection: None,
    })?;
    if workspace.document.selection != Some(Selection::rectangle(0, 0, EDGE, EDGE)) {
        bail!("uniform magic wand benchmark did not canonicalize to a rectangle");
    }
    let pixels = u64::from(EDGE) * u64::from(EDGE);
    // Calculated major-plane bound: exact composite RGBA plus alpha, visited,
    // and AA-core planes. Arc-backed command/document clones do not copy alpha.
    // The logical budget reserves another eight MiB for the flood frontier and
    // compositor bookkeeping. This is a calculated bound, not an RSS sample.
    Ok(MagicWandSample {
        elapsed_ms: started.elapsed().as_secs_f64() * 1_000.0,
        major_plane_bytes: pixels * 7,
        logical_peak_bytes: pixels * 7 + 8 * 1024 * 1024,
    })
}

pub(super) fn measure_selection_fill() -> Result<SelectionFillSamples> {
    let mut samples = Vec::with_capacity(17);
    for _ in 0..17 {
        let mut workspace = Workspace::new(Document::new("Selection fill", 4_096, 4_096), None);
        workspace.execute(Command::SetSelection {
            selection: Some(Selection::rectangle(1_024, 960, 640, 480)),
        })?;
        let started = Instant::now();
        workspace.execute(Command::FillSelection {
            color: [93, 216, 199, 220],
            name: None,
        })?;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        let fill = workspace
            .document
            .layers
            .last()
            .context("selection fill benchmark did not create a layer")?;
        if !matches!(
            fill.kind,
            LayerKind::Rectangle {
                width: 640,
                height: 480,
                color: [93, 216, 199, 220],
                corner_radius: 0.0,
            }
        ) {
            bail!("selection fill benchmark created the wrong editable layer");
        }
    }
    samples.sort_by(f64::total_cmp);
    Ok(SelectionFillSamples {
        median_ms: samples[samples.len() / 2],
        p95_ms: samples[16],
    })
}
