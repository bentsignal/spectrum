//! Snapping a moving layer to guides, the canvas's edges and center, and
//! other layers' edges and centers. Clients supply layer bounds and decide
//! the tolerance in canvas units (a few screen pixels at the current zoom).
use crate::{Document, GuideOrientation};

/// Where a move lands after snapping, and the lines it snapped to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snapped {
    pub dx: f32,
    pub dy: f32,
    /// The x of a vertical line the layer lines up with.
    pub vertical: Option<f32>,
    /// The y of a horizontal line the layer lines up with.
    pub horizontal: Option<f32>,
}

/// Lines worth snapping to: guides, the canvas's edges and center, and the
/// edges and centers of `others` (bounds of the layers not moving).
pub fn snap_targets(
    document: &Document,
    others: impl IntoIterator<Item = ([f32; 2], [f32; 2])>,
) -> (Vec<f32>, Vec<f32>) {
    let (width, height) = (document.width as f32, document.height as f32);
    let mut x = vec![0., width / 2., width];
    let mut y = vec![0., height / 2., height];
    for guide in &document.guides {
        match guide.orientation {
            GuideOrientation::Vertical => x.push(guide.position),
            GuideOrientation::Horizontal => y.push(guide.position),
        }
    }
    for (min, max) in others {
        x.extend([min[0], (min[0] + max[0]) / 2., max[0]]);
        y.extend([min[1], (min[1] + max[1]) / 2., max[1]]);
    }
    (x, y)
}

/// The nearest line within `tolerance` of any of `edges`, as (shift, line).
fn closest(edges: [f32; 3], targets: &[f32], tolerance: f32) -> Option<(f32, f32)> {
    edges
        .iter()
        .flat_map(|edge| targets.iter().map(move |target| (target - edge, *target)))
        .filter(|(shift, _)| shift.abs() <= tolerance)
        .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
}

/// Moves `bounds` by (`dx`, `dy`), then nudges it so an edge or its center
/// lands on the nearest target line within `tolerance` on each axis.
pub fn snap_move(
    (min, max): ([f32; 2], [f32; 2]),
    dx: f32,
    dy: f32,
    (targets_x, targets_y): (&[f32], &[f32]),
    tolerance: f32,
) -> Snapped {
    let edges = |a: f32, b: f32| [a, (a + b) / 2., b];
    let x = closest(edges(min[0] + dx, max[0] + dx), targets_x, tolerance);
    let y = closest(edges(min[1] + dy, max[1] + dy), targets_y, tolerance);
    Snapped {
        dx: dx + x.map_or(0., |(shift, _)| shift),
        dy: dy + y.map_or(0., |(shift, _)| shift),
        vertical: x.map(|(_, line)| line),
        horizontal: y.map(|(_, line)| line),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_an_edge_or_center_to_the_nearest_line_within_tolerance() {
        let doc = Document::new("Snap", 1000, 500);
        let targets = snap_targets(&doc, [([300., 100.], [400., 200.])]);
        let targets = (targets.0.as_slice(), targets.1.as_slice());
        // A 100-wide layer at x 10..110 moved 192 right: its right edge at
        // 302 is 2 from the other layer's left edge at 300.
        let snapped = snap_move(([10., 10.], [110., 60.]), 192., 0., targets, 5.);
        assert_eq!(snapped.dx, 190.);
        assert_eq!(snapped.vertical, Some(300.));
        // Too far from any line: the move is unchanged.
        let free = snap_move(([10., 10.], [110., 60.]), 120., 0., targets, 5.);
        assert_eq!(free.dx, 120.);
        assert_eq!(free.vertical, None);
        // Centers snap too: a 100-tall layer centered near the canvas middle.
        let canvas_only = snap_targets(&doc, []);
        let canvas_only = (canvas_only.0.as_slice(), canvas_only.1.as_slice());
        let centered = snap_move(([0., 196.], [50., 296.]), 0., 1., canvas_only, 5.);
        assert_eq!(centered.dy, 4.);
        assert_eq!(centered.horizontal, Some(250.));
    }
}
