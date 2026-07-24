use std::{f32::consts::TAU, time::Instant};

use anyhow::{Result, bail};
use prism_core::{
    Command, Document, GradientStop, Layer, LayerKind, PathAnchor, PathFillRule, PathGeometry,
    RenderRegion, ShapeFill, ShapeGradient, ShapeStroke, Workspace, path_preview_requires_region,
    render_document_region_scaled_with_stats,
};

const ANCHOR_COUNT: usize = 256;
const VIEWPORT_SIZE: u32 = 2_048;
const HIGH_ZOOM: f32 = 16.0;

pub(super) struct PathMeasurements {
    pub raster_median_ms: f64,
    pub raster_p95_ms: f64,
    pub edit_median_ms: f64,
    pub edit_p95_ms: f64,
}

pub(super) fn measure() -> Result<PathMeasurements> {
    let geometry = benchmark_geometry()?;
    let layer = Layer {
        id: 1,
        shape_fill: Some(ShapeFill::Gradient(ShapeGradient {
            angle: 31.0,
            stops: vec![
                GradientStop::new(0.0, [53, 201, 220, 238]),
                GradientStop::new(1.0, [188, 74, 231, 214]),
            ],
            ..ShapeGradient::default()
        })),
        stroke: ShapeStroke {
            enabled: true,
            width: 2.0,
            color: [246, 224, 132, 255],
        },
        kind: LayerKind::Path {
            geometry: geometry.clone(),
            color: [53, 201, 220, 238],
        },
        ..Layer::default()
    };

    if !path_preview_requires_region(&layer, HIGH_ZOOM)? {
        bail!("16x path benchmark did not activate viewport-bounded rendering");
    }
    let mut document = Document::new("Path viewport benchmark", 4_096, 4_096);
    document.layers.push(layer);
    let region = RenderRegion {
        x: 23_000,
        y: 23_000,
        width: 320,
        height: 180,
    };
    let (warm, warm_stats) =
        render_document_region_scaled_with_stats(&document, HIGH_ZOOM, region)?;
    if (warm.width(), warm.height()) != (region.width, region.height)
        || warm_stats.max_source_staging_pixels > 4_096 * 4_096
    {
        bail!("16x path viewport benchmark exceeded its bounded region");
    }
    let mut raster_samples = Vec::with_capacity(5);
    for _ in 0..5 {
        let started = Instant::now();
        let (rendered, stats) =
            render_document_region_scaled_with_stats(&document, HIGH_ZOOM, region)?;
        if (rendered.width(), rendered.height()) != (region.width, region.height)
            || stats.max_source_staging_pixels > 4_096 * 4_096
        {
            bail!("16x path viewport raster exceeded its bounded region");
        }
        raster_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }

    let mut workspace = Workspace::new(Document::new("Path edit benchmark", 512, 512), None);
    workspace.execute(Command::AddPath {
        name: Some("256-anchor cubic path".into()),
        geometry: geometry.clone(),
        color: [53, 201, 220, 238],
        x: 128.0,
        y: 128.0,
    })?;
    let id = workspace.document.selected.expect("added path is selected");
    workspace.begin_interaction();
    let mut edit_samples = Vec::with_capacity(240);
    for frame in 0..240 {
        let started = Instant::now();
        let mut anchor = geometry.anchors()[frame % ANCHOR_COUNT];
        anchor.point[0] += (frame as f32 * 0.07).sin() * 3.0;
        anchor.point[1] += (frame as f32 * 0.11).cos() * 3.0;
        let preview = geometry.replacing_anchor(frame % ANCHOR_COUNT, anchor)?;
        workspace.preview(Command::ReplacePath {
            id,
            geometry: preview,
        })?;
        edit_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    workspace.commit_interaction()?;

    let (raster_median_ms, raster_p95_ms) = sample_summary(&mut raster_samples);
    let (edit_median_ms, edit_p95_ms) = sample_summary(&mut edit_samples);
    Ok(PathMeasurements {
        raster_median_ms,
        raster_p95_ms,
        edit_median_ms,
        edit_p95_ms,
    })
}

fn benchmark_geometry() -> Result<PathGeometry> {
    let center = VIEWPORT_SIZE as f32 / 2.0;
    let radius = 54.0;
    let step = TAU / ANCHOR_COUNT as f32;
    let handle_length = radius * 4.0 / 3.0 * (step / 4.0).tan();
    let anchors = (0..ANCHOR_COUNT)
        .map(|index| {
            let angle = index as f32 * step;
            let (sin, cos) = angle.sin_cos();
            let tangent = [-sin * handle_length, cos * handle_length];
            PathAnchor {
                point: [center + cos * radius, center + sin * radius],
                handle_in: [-tangent[0], -tangent[1]],
                handle_out: tangent,
            }
        })
        .collect::<Vec<_>>();
    PathGeometry::new(
        VIEWPORT_SIZE,
        VIEWPORT_SIZE,
        true,
        PathFillRule::EvenOdd,
        anchors,
    )
}

fn sample_summary(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    let p95 = ((samples.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(samples.len() - 1);
    (median, samples[p95])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_path_is_maximally_bounded() {
        let geometry = benchmark_geometry().unwrap();
        assert_eq!(geometry.anchors().len(), prism_core::MAX_PATH_ANCHORS);
        assert_eq!((geometry.width(), geometry.height()), (2_048, 2_048));
        assert!(geometry.closed());
    }
}
