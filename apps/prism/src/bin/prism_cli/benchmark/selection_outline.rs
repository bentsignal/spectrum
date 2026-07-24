use std::time::Instant;

use anyhow::{Result, bail};
use prism_core::{
    MAX_SELECTION_OUTLINE_EDGES, SelectionMaskOutline, SelectionOutlinePoint, SelectionOutlineRect,
    SelectionOutlineTransform, SelectionOutlineView, complex_selection_mask_outline,
    marching_ants_frame, selection_mask_outline,
};

pub(super) struct SelectionOutlineSamples {
    pub(super) median_ms: f64,
    pub(super) p95_ms: f64,
}

pub(super) fn measure() -> Result<SelectionOutlineSamples> {
    const EDGE: u32 = 1_024;
    let mut alpha = vec![0; (EDGE * EDGE) as usize];
    for y in 0..EDGE {
        for x in 0..EDGE {
            alpha[(y * EDGE + x) as usize] = if (x + y) % 2 == 0 { 255 } else { 0 };
        }
    }

    let mut samples = Vec::with_capacity(9);
    for sample in 0..9 {
        let started = Instant::now();
        if !matches!(
            selection_mask_outline((0, 0, EDGE, EDGE), &alpha),
            SelectionMaskOutline::Complex
        ) {
            bail!("pathological selection outline no longer exercises the bounded path");
        }
        let paths = complex_selection_mask_outline(
            (0, 0, EDGE, EDGE),
            &alpha,
            SelectionOutlineView {
                x: 0,
                y: 0,
                width: EDGE,
                height: EDGE,
            },
        );
        let frame = marching_ants_frame(
            &paths,
            SelectionOutlineTransform {
                scale: 1.0,
                offset: SelectionOutlinePoint::new(0.0, 0.0),
            },
            SelectionOutlineRect::new(
                SelectionOutlinePoint::new(0.0, 0.0),
                SelectionOutlinePoint::new(1_024.0, 768.0),
            ),
            sample as f32,
            4.0,
        );
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        let edges = paths
            .iter()
            .map(|path| path.len().saturating_sub(1))
            .sum::<usize>();
        if paths.len() <= 1
            || edges > MAX_SELECTION_OUTLINE_EDGES
            || frame.contrast.is_empty()
            || frame.light.is_empty()
        {
            bail!("bounded selection outline lost mixed-mask or animated-frame work");
        }
    }

    let zoomed = complex_selection_mask_outline(
        (0, 0, EDGE, EDGE),
        &alpha,
        SelectionOutlineView {
            x: 400,
            y: 300,
            width: 64,
            height: 48,
        },
    );
    if zoomed.len() != 64 * 48 / 2 * 4 || zoomed.iter().any(|path| path.len() != 2) {
        bail!("high-zoom selection outline did not retain exact source-pixel edges");
    }

    samples.sort_by(f64::total_cmp);
    Ok(SelectionOutlineSamples {
        median_ms: samples[samples.len() / 2],
        p95_ms: *samples
            .last()
            .expect("selection outline samples are nonempty"),
    })
}
