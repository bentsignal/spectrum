//! Selection helpers for clients: the inverse of a selection, a selection's
//! outline as a layer's vector mask, and an ellipse as a lasso path. Each
//! returns a value for an existing command (`SetSelection`, `SetVectorMask`,
//! `LassoSelection`), so its result is saved and undone like any other edit.
use anyhow::{Result, bail};

use crate::{
    Document, LassoPath, LassoPoint, PathAnchor, PathFillRule, PathGeometry, Selection,
    SelectionMaskOutline, VectorMask, selection_mask_outline,
};

/// Every canvas pixel the current selection leaves out; with no selection,
/// the whole canvas.
pub fn inverted_selection(document: &Document) -> Result<Selection> {
    let (width, height) = (document.width, document.height);
    let Some(selection) = &document.selection else {
        return Ok(Selection::rectangle(0, 0, width, height));
    };
    let pixels = u64::from(width) * u64::from(height);
    if pixels > crate::MAX_COLOR_SELECTION_PIXELS {
        bail!(
            "inverting a selection is bounded to {} canvas pixels",
            crate::MAX_COLOR_SELECTION_PIXELS
        );
    }
    let mut alpha = vec![255u8; pixels as usize];
    let (x, y, w, h) = selection.bounds();
    for row in 0..h {
        let canvas_y = y + row;
        if canvas_y >= height {
            break;
        }
        for column in 0..w {
            let canvas_x = x + column;
            if canvas_x >= width {
                break;
            }
            let selected = selection
                .alpha()
                .map_or(255, |mask| mask[(row * w + column) as usize]);
            alpha[(canvas_y * width + canvas_x) as usize] = 255 - selected;
        }
    }
    Ok(Selection::color_mask(0, 0, width, height, alpha))
}

/// The selection's outline in canvas pixels: its largest closed contour.
pub fn selection_outline_polygon(selection: &Selection) -> Vec<[f32; 2]> {
    let (x, y, w, h) = selection.bounds();
    let Some(alpha) = selection.alpha() else {
        let (x, y, w, h) = (x as f32, y as f32, w as f32, h as f32);
        return vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    };
    let SelectionMaskOutline::Exact(paths) = selection_mask_outline((x, y, w, h), alpha) else {
        let (x, y, w, h) = (x as f32, y as f32, w as f32, h as f32);
        return vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    };
    let area = |path: &Vec<crate::SelectionOutlinePoint>| {
        path.windows(2)
            .map(|p| p[0].x * p[1].y - p[1].x * p[0].y)
            .sum::<f32>()
            .abs()
    };
    paths
        .iter()
        .max_by(|a, b| area(a).total_cmp(&area(b)))
        .map(|path| path.iter().map(|p| [p.x, p.y]).collect())
        .unwrap_or_default()
}

/// A vector mask on layer `id` that shows only what the selection covers,
/// following the layer's position, scale, and rotation.
pub fn vector_mask_from_selection(document: &Document, id: u64) -> Result<VectorMask> {
    let Some(selection) = &document.selection else {
        bail!("there is no selection to mask with");
    };
    let layer = document.layer(id)?;
    let geometry = crate::document_layer_geometry(document, layer)?;
    let (scale_x, scale_y) = (
        layer.transform.scale_x.abs().max(1e-6),
        layer.transform.scale_y.abs().max(1e-6),
    );
    // The layer's own size before scaling: what the mask's viewport spans.
    let (sin, cos) = crate::transform_math::rotation_sin_cos(layer.transform.rotation);
    let corner = |i: usize| geometry.corners[i];
    let width = (corner(1)[0] - corner(0)[0]).hypot(corner(1)[1] - corner(0)[1]) / scale_x;
    let height = (corner(3)[0] - corner(0)[0]).hypot(corner(3)[1] - corner(0)[1]) / scale_y;
    if width < 1.0 || height < 1.0 {
        bail!("layer {id} is too small to mask");
    }
    // The mask's viewport is whole pixels; it stretches over the layer.
    let (width, height) = (width.round(), height.round());
    let to_layer = |[x, y]: [f32; 2]| {
        let (dx, dy) = (x - geometry.center[0], y - geometry.center[1]);
        // Undo the rotation, then the scale, about the layer's center.
        let (rx, ry) = (dx * cos + dy * sin, -dx * sin + dy * cos);
        [rx / scale_x + width / 2.0, ry / scale_y + height / 2.0]
    };
    let polygon = selection_outline_polygon(selection)
        .into_iter()
        .map(to_layer)
        .collect();
    // Only the part over the layer matters, and anchors stay in its box.
    let anchors: Vec<PathAnchor> =
        simplified(clip_polygon(polygon, width, height), MAX_MASK_ANCHORS)
            .into_iter()
            .map(|[x, y]| PathAnchor::corner(x, y))
            .collect();
    if anchors.len() < 3 {
        bail!("the selection has no area to mask with");
    }
    let path = PathGeometry::new(
        width as u32,
        height as u32,
        true,
        PathFillRule::EvenOdd,
        anchors,
    )?;
    VectorMask::new(path, false)
}

/// The most anchors a path holds.
const MAX_MASK_ANCHORS: usize = 256;

/// `polygon` with points dropped (Ramer, Douglas, and Peucker) until at most
/// `limit` remain, loosening the tolerance from a quarter pixel.
fn simplified(polygon: Vec<[f32; 2]>, limit: usize) -> Vec<[f32; 2]> {
    fn keep(points: &[[f32; 2]], tolerance: f32, out: &mut Vec<[f32; 2]>) {
        let (first, last) = (points[0], points[points.len() - 1]);
        let (dx, dy) = (last[0] - first[0], last[1] - first[1]);
        let length = dx.hypot(dy).max(1e-6);
        let (index, distance) = points[1..points.len() - 1]
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    i + 1,
                    ((p[0] - first[0]) * dy - (p[1] - first[1]) * dx).abs() / length,
                )
            })
            .fold(
                (0, 0.0f32),
                |best, next| if next.1 > best.1 { next } else { best },
            );
        if distance > tolerance && index > 0 {
            keep(&points[..=index], tolerance, out);
            keep(&points[index..], tolerance, out);
        } else {
            out.push(first);
        }
    }
    let mut tolerance = 0.25;
    let mut current = polygon;
    while current.len() > limit {
        // A closed loop starts and ends at one point; split it at the point
        // farthest from there so each half has a chord to measure from.
        let start = current[0];
        let far = (1..current.len())
            .max_by(|&a, &b| {
                let d = |p: [f32; 2]| (p[0] - start[0]).hypot(p[1] - start[1]);
                d(current[a]).total_cmp(&d(current[b]))
            })
            .unwrap_or(0);
        let mut out = Vec::new();
        keep(&current[..=far], tolerance, &mut out);
        let mut back = current[far..].to_vec();
        back.push(start);
        keep(&back, tolerance, &mut out);
        current = out;
        tolerance *= 2.0;
    }
    current
}

/// `polygon` cut to the box from (0, 0) to (`width`, `height`), one edge at
/// a time (Sutherland and Hodgman).
fn clip_polygon(mut polygon: Vec<[f32; 2]>, width: f32, height: f32) -> Vec<[f32; 2]> {
    let edges: [(usize, f32, bool); 4] = [
        (0, 0.0, true),
        (0, width, false),
        (1, 0.0, true),
        (1, height, false),
    ];
    for (axis, limit, lower) in edges {
        let inside = |p: [f32; 2]| {
            if lower {
                p[axis] >= limit
            } else {
                p[axis] <= limit
            }
        };
        let mut out = Vec::with_capacity(polygon.len() + 4);
        for (i, &current) in polygon.iter().enumerate() {
            let previous = polygon[(i + polygon.len() - 1) % polygon.len()];
            let cross = |a: [f32; 2], b: [f32; 2]| {
                let t = (limit - a[axis]) / (b[axis] - a[axis]);
                [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
            };
            match (inside(previous), inside(current)) {
                (true, true) => out.push(current),
                (true, false) => out.push(cross(previous, current)),
                (false, true) => {
                    out.push(cross(previous, current));
                    out.push(current);
                }
                (false, false) => {}
            }
        }
        polygon = out;
        if polygon.is_empty() {
            break;
        }
    }
    // Rounding can leave a point a hair outside.
    polygon
        .into_iter()
        .map(|[x, y]| [x.clamp(0.0, width), y.clamp(0.0, height)])
        .collect()
}

/// An ellipse inscribed in the box from `start` to `end`, as a lasso path.
pub fn ellipse_lasso(start: (f32, f32), end: (f32, f32)) -> Result<LassoPath> {
    let center = ((start.0 + end.0) / 2.0, (start.1 + end.1) / 2.0);
    let radius = ((end.0 - start.0).abs() / 2.0, (end.1 - start.1).abs() / 2.0);
    let steps = ((radius.0 + radius.1) * 0.75).clamp(24.0, 256.0) as usize;
    let points = (0..steps)
        .map(|i| {
            let t = i as f32 / steps as f32 * std::f32::consts::TAU;
            LassoPoint::from_canvas(center.0 + radius.0 * t.cos(), center.1 + radius.1 * t.sin())
        })
        .collect::<Result<Vec<_>>>()?;
    LassoPath::new(points)
}

/// The box from `start` to `end` as a lasso path, so it combines with the
/// current selection like any lasso.
pub fn rectangle_lasso(start: (f32, f32), end: (f32, f32)) -> Result<LassoPath> {
    let (x0, x1) = (start.0.min(end.0), start.0.max(end.0));
    let (y0, y1) = (start.1.min(end.1), start.1.max(end.1));
    LassoPath::new(
        [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
            .into_iter()
            .map(|(x, y)| LassoPoint::from_canvas(x, y))
            .collect::<Result<Vec<_>>>()?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Layer, LayerKind, SelectionCombineMode, Workspace};

    fn document() -> Document {
        let mut document = Document::new("Selections", 40, 30);
        document.layers.push(Layer {
            id: 1,
            kind: LayerKind::Rectangle {
                width: 20,
                height: 10,
                color: [255, 0, 0, 255],
                corner_radius: 0.0,
            },
            transform: crate::Transform {
                x: 10.0,
                y: 10.0,
                ..crate::Transform::default()
            },
            ..Layer::default()
        });
        document.next_id = 2;
        document
    }

    #[test]
    fn inverting_twice_restores_the_selected_pixels() {
        let mut document = document();
        document.selection = Some(Selection::rectangle(5, 5, 10, 10));
        let inverted = inverted_selection(&document).unwrap();
        let alpha = inverted.alpha().unwrap();
        assert_eq!(alpha[6 * 40 + 6], 0);
        assert_eq!(alpha[0], 255);
        document.selection = Some(inverted);
        let back = inverted_selection(&document).unwrap();
        assert_eq!(back.alpha().unwrap()[6 * 40 + 6], 255);
        assert_eq!(back.alpha().unwrap()[0], 0);
    }

    #[test]
    fn selection_masks_the_layer_where_it_was_drawn() {
        let mut workspace = Workspace::new(document(), None);
        let lasso = rectangle_lasso((20.0, 0.0), (40.0, 30.0)).unwrap();
        workspace
            .execute(Command::LassoSelection {
                points: lasso,
                mode: SelectionCombineMode::Replace,
                antialias: false,
            })
            .unwrap();
        let mask = vector_mask_from_selection(&workspace.document, 1).unwrap();
        workspace
            .execute(Command::SetVectorMask {
                id: 1,
                mask: Some(mask),
            })
            .unwrap();
        let image = crate::render_document(&workspace.document, None)
            .unwrap()
            .to_rgba8();
        assert_eq!(
            image.get_pixel(15, 15)[0],
            document().background[0],
            "masked out"
        );
        assert_eq!(image.get_pixel(25, 15).0, [255, 0, 0, 255], "kept");
        assert!(ellipse_lasso((0.0, 0.0), (10.0, 6.0)).is_ok());
        let circle: Vec<[f32; 2]> = (0..2000)
            .map(|i| {
                let t = i as f32 / 2000.0 * std::f32::consts::TAU;
                [50.0 + 40.0 * t.cos(), 50.0 + 40.0 * t.sin()]
            })
            .collect();
        let fewer = simplified(circle, MAX_MASK_ANCHORS);
        assert!(fewer.len() <= MAX_MASK_ANCHORS && fewer.len() > 16);
        // Every kept point is still on the circle, all the way round.
        for p in &fewer {
            assert!(((p[0] - 50.0).hypot(p[1] - 50.0) - 40.0).abs() < 0.01);
        }
        let spread = fewer.iter().map(|p| p[1]).fold(f32::MIN, f32::max)
            - fewer.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert!(spread > 79.0, "no part of the loop is cut off");
    }
}
