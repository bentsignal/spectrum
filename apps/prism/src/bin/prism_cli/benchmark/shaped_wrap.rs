use std::time::Instant;

use anyhow::{Result, bail};
use prism_core::{TextShaping, TextTypography, measure_text_geometry_with_typography};

pub(super) struct ShapedWrapMeasurement {
    pub(super) samples: Vec<f64>,
    pub(super) graphemes: usize,
    pub(super) break_opportunities: usize,
}

pub(super) fn measure() -> Result<ShapedWrapMeasurement> {
    let mut text = "1111 ".repeat(3_000);
    text.pop();
    let graphemes = text.chars().count();
    let break_opportunities = unicode_linebreak::linebreaks(&text).count();
    if graphemes != 14_999 || !(2_999..=3_001).contains(&break_opportunities) {
        bail!("wide shaped-wrap benchmark fixture changed");
    }
    let typography = TextTypography {
        box_width: Some(1_000_000.0),
        shaping: TextShaping::harfbuzz_v1(Some("en"))?,
        ..Default::default()
    };
    let mut samples = Vec::with_capacity(7);
    for _ in 0..7 {
        let started = Instant::now();
        let geometry = measure_text_geometry_with_typography(&text, 20.0, &typography, None)?;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        if geometry.layout_height > 32.0 {
            bail!("wide shaped-wrap benchmark unexpectedly produced multiple lines");
        }
    }
    Ok(ShapedWrapMeasurement {
        samples,
        graphemes,
        break_opportunities,
    })
}
