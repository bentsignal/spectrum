use std::{hint::black_box, time::Instant};

use prism_core::font_metadata_matches_query;

use super::sample_summary;

const LARGE_FONT_COLLECTION_SIZE: usize = 10_000;
const BOUNDED_VISIBLE_RESULTS: usize = 512;

pub(super) struct Measurement {
    pub(super) median_ms: f64,
    pub(super) p95_ms: f64,
}

pub(super) fn measure() -> Measurement {
    let faces = (0..LARGE_FONT_COLLECTION_SIZE)
        .map(|index| {
            (
                format!("Collection Family {index:05}"),
                if index % 4 == 0 {
                    "Bold Italic"
                } else {
                    "Regular"
                },
                if index % 4 == 0 { 700 } else { 400 },
                if index % 4 == 0 { "Italic" } else { "Normal" },
            )
        })
        .collect::<Vec<_>>();
    let mut samples = Vec::with_capacity(60);
    for sample in 0..60 {
        let started = Instant::now();
        let mut matching_faces = faces
            .iter()
            .enumerate()
            .filter_map(|(id, (family, style, weight, slant))| {
                font_metadata_matches_query(family, style, *weight, slant, "collection")
                    .then_some((id, family, style, weight, slant))
            })
            .collect::<Vec<_>>();
        matching_faces.sort_by(|left, right| {
            left.1
                .cmp(right.1)
                .then_with(|| left.3.cmp(right.3))
                .then_with(|| left.2.cmp(right.2))
                .then_with(|| left.4.cmp(right.4))
                .then_with(|| left.0.cmp(&right.0))
        });
        let visible_faces = matching_faces
            .into_iter()
            .take(BOUNDED_VISIBLE_RESULTS)
            .collect::<Vec<_>>();
        // Hover dispatch retains only one reversible face id. It does not shape
        // or retain a preview for every search result, so bounded renderer
        // caches remain independent of collection size.
        let hover_preview_font_id = visible_faces
            .get(sample % visible_faces.len())
            .map(|face| face.0);
        black_box(hover_preview_font_id);
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let (median_ms, p95_ms) = sample_summary(&mut samples);
    Measurement { median_ms, p95_ms }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_collection_measurement_is_finite() {
        let measurement = measure();
        assert!(measurement.median_ms.is_finite());
        assert!(measurement.p95_ms >= measurement.median_ms);
    }
}
