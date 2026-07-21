use std::time::Instant;

use anyhow::{Context, Result, bail};
use prism_core::{Command, Document, LayerKind, Selection, Workspace};

pub(super) struct SelectionFillSamples {
    pub(super) median_ms: f64,
    pub(super) p95_ms: f64,
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
