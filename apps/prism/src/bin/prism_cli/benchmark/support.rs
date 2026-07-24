use std::{io::Write, path::PathBuf};

use anyhow::Result;
use clap::ValueEnum;
use serde::Serialize;

#[derive(Clone, Copy, Default, ValueEnum)]
pub(crate) enum BenchmarkProfile {
    #[default]
    Interactive,
    HostedCi,
}

impl BenchmarkProfile {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Interactive => "interactive-workstation",
            Self::HostedCi => "github-hosted-linux",
        }
    }

    pub(crate) fn gradient_shadow_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 500.0,
            // The reviewed implementation measured 880.788 ms p95 on GitHub's
            // shared Linux runner versus 222.508 ms locally. A 1,250 ms ceiling
            // keeps 42% host-jitter headroom while the original 2,061.886 ms
            // regression still fails decisively.
            Self::HostedCi => 1_250.0,
        }
    }

    pub(crate) fn magic_wand_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 5_000.0,
            Self::HostedCi => 15_000.0,
        }
    }

    pub(crate) fn path_raster_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 250.0,
            Self::HostedCi => 750.0,
        }
    }

    pub(crate) fn path_edit_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 5.0,
            Self::HostedCi => 15.0,
        }
    }

    pub(crate) fn paint_viewport_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 500.0,
            Self::HostedCi => 1_500.0,
        }
    }

    pub(crate) fn brush_drag_preview_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 5.0,
            Self::HostedCi => 15.0,
        }
    }
}

#[derive(Serialize)]
pub(super) struct BenchmarkMetric {
    pub(super) name: &'static str,
    pub(super) median_ms: f64,
    pub(super) p95_ms: f64,
    pub(super) budget_ms: f64,
    pub(super) pass: bool,
}

pub(super) struct TemporaryRaster {
    pub(super) path: PathBuf,
}

impl TemporaryRaster {
    pub(super) fn new(width: u32, height: u32) -> Result<Self> {
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

pub(super) fn sample_summary(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    let p95_index = ((samples.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
    (median, samples[p95_index.min(samples.len() - 1)])
}
