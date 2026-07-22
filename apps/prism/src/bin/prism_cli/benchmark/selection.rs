use std::time::Instant;

use anyhow::{Context, Result, bail};
use prism_core::{Command, Document, LayerKind, Selection, Workspace};

pub(super) struct SelectionFillSamples {
    pub(super) median_ms: f64,
    pub(super) p95_ms: f64,
}

pub(super) struct MagicWandSample {
    pub(super) elapsed_ms: f64,
    pub(super) major_plane_bytes: u64,
    pub(super) logical_peak_bytes: u64,
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
