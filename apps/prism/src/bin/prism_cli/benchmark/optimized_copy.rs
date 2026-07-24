use std::{fs, path::PathBuf, time::Instant};

use anyhow::{Context, Result, bail};
use prism_core::{Command, Document, TextTypography, Workspace, create_optimized_font_copy};
use spectrum_revisions::{Actor, ActorKind, SessionId};

const STATIC_FONT: &[u8] = include_bytes!(
    "../../../../../../crates/spectrum-fonts/tests/fonts/noto-sans-static-source.ttf"
);

pub(super) struct OptimizedCopyMeasurement {
    pub(super) samples: Vec<f64>,
    pub(super) reduction_bytes: u64,
}

pub(super) fn measure() -> Result<OptimizedCopyMeasurement> {
    let directory = fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!(
            "prism-optimized-copy-benchmark-{}",
            SessionId::new()
        ));
    fs::create_dir(&directory)?;
    let cleanup = Cleanup(directory.clone());
    let source = directory.join("source.prism");
    let font = directory.join("NotoSans.ttf");
    fs::write(&font, STATIC_FONT)?;
    let mut workspace = Workspace::create_durable(
        Document::new("Optimized copy benchmark", 640, 360),
        &source,
        Actor {
            id: "benchmark:optimized-copy".into(),
            display_name: "Optimized Copy Benchmark".into(),
            kind: ActorKind::System,
        },
        SessionId::new(),
    )?;
    workspace.execute(Command::ImportFont {
        path: font,
        source_name: None,
    })?;
    let font_id = workspace
        .document
        .font_assets
        .first()
        .context("benchmark font import did not create an asset")?
        .id;
    workspace.execute(Command::AddText {
        text: "AVBA".into(),
        name: None,
        font_size: 42.0,
        color: [255; 4],
        x: 16.0,
        y: 24.0,
    })?;
    workspace.execute(Command::SetTextTypography {
        id: workspace
            .document
            .selected
            .context("benchmark text is not selected")?,
        typography: TextTypography {
            font_id: Some(font_id),
            ..Default::default()
        },
    })?;
    drop(workspace);

    let mut samples = Vec::new();
    let mut reduction_bytes = 0;
    for index in 0..3 {
        let output = directory.join(format!("optimized-{index}.prism"));
        let started = Instant::now();
        let report = create_optimized_font_copy(&source, &output)?;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        if report.fonts.len() != 1
            || report.fonts[0].subset_bytes >= report.fonts[0].source_bytes
            || report.saved_bytes == 0
        {
            bail!("optimized-copy benchmark did not produce a verified reduction");
        }
        reduction_bytes = report.saved_bytes;
    }
    drop(cleanup);
    Ok(OptimizedCopyMeasurement {
        samples,
        reduction_bytes,
    })
}

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
