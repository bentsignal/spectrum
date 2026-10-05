use std::sync::Arc;

pub const MAX_SELECTION_OUTLINE_EDGES: usize = 65_536;
const MAX_COARSE_CELLS: usize = MAX_SELECTION_OUTLINE_EDGES / 4;
const SELECTION_BOUNDARY_ALPHA: u8 = 128;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionOutlinePoint {
    pub x: f32,
    pub y: f32,
}

impl SelectionOutlinePoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }

    fn distance(self, other: Self) -> f32 {
        (other.x - self.x).hypot(other.y - self.y)
    }
}

pub type SelectionOutlinePath = Vec<SelectionOutlinePoint>;

#[derive(Clone, Debug)]
pub enum SelectionMaskOutline {
    Exact(Arc<[SelectionOutlinePath]>),
    Complex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionOutlineView {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl SelectionOutlineView {
    pub fn clipped(self, width: u32, height: u32) -> Option<Self> {
        let x = self.x.min(width);
        let y = self.y.min(height);
        let right = self.x.saturating_add(self.width).min(width);
        let bottom = self.y.saturating_add(self.height).min(height);
        (right > x && bottom > y).then_some(Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionOutlineRect {
    pub min: SelectionOutlinePoint,
    pub max: SelectionOutlinePoint,
}

impl SelectionOutlineRect {
    pub const fn new(min: SelectionOutlinePoint, max: SelectionOutlinePoint) -> Self {
        Self { min, max }
    }

    pub fn expand(self, amount: f32) -> Self {
        Self::new(
            SelectionOutlinePoint::new(self.min.x - amount, self.min.y - amount),
            SelectionOutlinePoint::new(self.max.x + amount, self.max.y + amount),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionOutlineTransform {
    pub scale: f32,
    pub offset: SelectionOutlinePoint,
}

impl SelectionOutlineTransform {
    fn apply(self, point: SelectionOutlinePoint) -> SelectionOutlinePoint {
        SelectionOutlinePoint::new(
            self.offset.x + point.x * self.scale,
            self.offset.y + point.y * self.scale,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionOutlineSegment {
    pub start: SelectionOutlinePoint,
    pub end: SelectionOutlinePoint,
}

#[derive(Clone, Debug, Default)]
pub struct SelectionOutlineFrame {
    pub contrast: Vec<SelectionOutlineSegment>,
    pub light: Vec<SelectionOutlineSegment>,
}

#[derive(Clone, Copy, Debug)]
struct BoundaryEdge {
    start: (u32, u32),
    end: (u32, u32),
    direction: u8,
}

pub fn selection_mask_outline(bounds: (u32, u32, u32, u32), alpha: &[u8]) -> SelectionMaskOutline {
    let (_, _, width, height) = bounds;
    if !valid_alpha(width, height, alpha) {
        return SelectionMaskOutline::Exact(Arc::from([]));
    }
    let view = SelectionOutlineView {
        x: 0,
        y: 0,
        width,
        height,
    };
    match boundary_edges(width, height, view, alpha) {
        Some(edges) => SelectionMaskOutline::Exact(map_contours(trace_contours(edges), bounds)),
        None => SelectionMaskOutline::Complex,
    }
}

pub fn complex_selection_mask_outline(
    bounds: (u32, u32, u32, u32),
    alpha: &[u8],
    view: SelectionOutlineView,
) -> Arc<[SelectionOutlinePath]> {
    let (_, _, width, height) = bounds;
    if !valid_alpha(width, height, alpha) {
        return Arc::from([]);
    }
    let Some(view) = view.clipped(width, height) else {
        return Arc::from([]);
    };
    if let Some(edges) = boundary_edges(width, height, view, alpha) {
        return edges
            .into_iter()
            .map(|edge| vec![map_point(bounds, edge.start), map_point(bounds, edge.end)])
            .collect();
    }
    coarse_mixed_outline(bounds, alpha, view)
}

pub fn marching_ants_frame(
    paths: &[SelectionOutlinePath],
    transform: SelectionOutlineTransform,
    clip: SelectionOutlineRect,
    phase: f32,
    dash_points: f32,
) -> SelectionOutlineFrame {
    let mut frame = SelectionOutlineFrame {
        contrast: Vec::with_capacity(paths.len()),
        light: Vec::with_capacity(paths.len()),
    };
    let cycle = dash_points * 2.0;
    if !cycle.is_finite() || cycle <= 0.0 {
        return frame;
    }
    for path in paths {
        let mut path_distance = 0.0;
        for segment in path.windows(2) {
            let start = transform.apply(segment[0]);
            let end = transform.apply(segment[1]);
            let length = start.distance(end);
            if !length.is_finite() || length <= f32::EPSILON {
                continue;
            }
            if let Some((clip_start, clip_end)) = clipped_segment_parameter(start, end, clip) {
                frame.contrast.push(SelectionOutlineSegment {
                    start: start.lerp(end, clip_start),
                    end: start.lerp(end, clip_end),
                });
                let mut distance = path_distance + clip_start * length;
                let visible_end = path_distance + clip_end * length;
                while distance < visible_end {
                    let cycle_position = (distance + phase).rem_euclid(cycle);
                    let light = cycle_position < dash_points;
                    let run = if light {
                        dash_points - cycle_position
                    } else {
                        cycle - cycle_position
                    };
                    let next = (distance + run.max(0.01)).min(visible_end);
                    if light {
                        let start_t = ((distance - path_distance) / length).clamp(0.0, 1.0);
                        let end_t = ((next - path_distance) / length).clamp(0.0, 1.0);
                        frame.light.push(SelectionOutlineSegment {
                            start: start.lerp(end, start_t),
                            end: start.lerp(end, end_t),
                        });
                    }
                    distance = next;
                }
            }
            path_distance += length;
        }
    }
    frame
}

fn valid_alpha(width: u32, height: u32, alpha: &[u8]) -> bool {
    width != 0
        && height != 0
        && u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| usize::try_from(pixels).ok())
            == Some(alpha.len())
}

fn selected(alpha: &[u8], width: u32, x: u32, y: u32) -> bool {
    alpha[(u64::from(y) * u64::from(width) + u64::from(x)) as usize] >= SELECTION_BOUNDARY_ALPHA
}

fn boundary_edges(
    width: u32,
    height: u32,
    view: SelectionOutlineView,
    alpha: &[u8],
) -> Option<Vec<BoundaryEdge>> {
    let mut edges = Vec::new();
    let mut push = |start, end, direction| {
        if edges.len() >= MAX_SELECTION_OUTLINE_EDGES {
            return false;
        }
        edges.push(BoundaryEdge {
            start,
            end,
            direction,
        });
        true
    };
    let right = view.x + view.width;
    let bottom = view.y + view.height;
    for y in view.y..bottom {
        for x in view.x..right {
            if !selected(alpha, width, x, y) {
                continue;
            }
            if y == 0 && !push((x, y), (x + 1, y), 0)
                || y > 0 && !selected(alpha, width, x, y - 1) && !push((x, y), (x + 1, y), 0)
                || x + 1 == width && !push((x + 1, y), (x + 1, y + 1), 1)
                || x + 1 < width
                    && !selected(alpha, width, x + 1, y)
                    && !push((x + 1, y), (x + 1, y + 1), 1)
                || y + 1 == height && !push((x + 1, y + 1), (x, y + 1), 2)
                || y + 1 < height
                    && !selected(alpha, width, x, y + 1)
                    && !push((x + 1, y + 1), (x, y + 1), 2)
                || x == 0 && !push((x, y + 1), (x, y), 3)
                || x > 0 && !selected(alpha, width, x - 1, y) && !push((x, y + 1), (x, y), 3)
            {
                return None;
            }
        }
    }
    Some(edges)
}

fn trace_contours(edges: Vec<BoundaryEdge>) -> Vec<Vec<(u32, u32)>> {
    use std::collections::HashMap;

    let mut outgoing: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (index, edge) in edges.iter().enumerate() {
        outgoing.entry(edge.start).or_default().push(index);
    }
    let mut visited = vec![false; edges.len()];
    let mut contours = Vec::new();
    for first in 0..edges.len() {
        if visited[first] {
            continue;
        }
        let start = edges[first].start;
        let mut points = vec![start];
        let mut current = first;
        loop {
            if visited[current] {
                break;
            }
            visited[current] = true;
            let edge = edges[current];
            points.push(edge.end);
            if edge.end == start {
                break;
            }
            let Some(candidates) = outgoing.get(&edge.end) else {
                break;
            };
            let Some(next) = candidates
                .iter()
                .copied()
                .filter(|candidate| !visited[*candidate])
                .min_by_key(|candidate| turn_rank(edge.direction, edges[*candidate].direction))
            else {
                break;
            };
            current = next;
        }
        let points = simplify_contour(points);
        if points.len() >= 4 && points.first() == points.last() {
            contours.push(points);
        }
    }
    contours
}

fn turn_rank(previous: u8, next: u8) -> u8 {
    match next.wrapping_sub(previous) % 4 {
        1 => 0,
        0 => 1,
        3 => 2,
        _ => 3,
    }
}

fn simplify_contour(points: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    if points.len() < 4 {
        return points;
    }
    let mut simplified = Vec::with_capacity(points.len());
    simplified.push(points[0]);
    for window in points.windows(3) {
        let first = window[0];
        let middle = window[1];
        let last = window[2];
        let collinear = (first.0 == middle.0 && middle.0 == last.0)
            || (first.1 == middle.1 && middle.1 == last.1);
        if !collinear {
            simplified.push(middle);
        }
    }
    simplified.push(*points.last().expect("contour has points"));
    simplified
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoarseCell {
    Empty,
    Full,
    Mixed,
}

fn coarse_mixed_outline(
    bounds: (u32, u32, u32, u32),
    alpha: &[u8],
    view: SelectionOutlineView,
) -> Arc<[SelectionOutlinePath]> {
    let (_, _, width, _) = bounds;
    let mut span = ((u64::from(view.width) * u64::from(view.height))
        .div_ceil(MAX_COARSE_CELLS as u64) as f64)
        .sqrt()
        .ceil()
        .max(1.0) as u32;
    while usize::try_from(view.width.div_ceil(span))
        .ok()
        .and_then(|columns| {
            usize::try_from(view.height.div_ceil(span))
                .ok()
                .and_then(|rows| columns.checked_mul(rows))
        })
        .is_none_or(|cells| cells > MAX_COARSE_CELLS)
    {
        span = span.saturating_add(1);
    }
    let columns = view.width.div_ceil(span);
    let rows = view.height.div_ceil(span);
    let mut cells = Vec::with_capacity((u64::from(columns) * u64::from(rows)) as usize);
    for row in 0..rows {
        let top = view.y + row * span;
        let bottom = (top + span).min(view.y + view.height);
        for column in 0..columns {
            let left = view.x + column * span;
            let right = (left + span).min(view.x + view.width);
            let mut count = 0_u64;
            for y in top..bottom {
                for x in left..right {
                    count += u64::from(selected(alpha, width, x, y));
                }
            }
            let area = u64::from(right - left) * u64::from(bottom - top);
            cells.push(if count == 0 {
                CoarseCell::Empty
            } else if count == area {
                CoarseCell::Full
            } else {
                CoarseCell::Mixed
            });
        }
    }

    let cell_at = |column: u32, row: u32| {
        cells[(u64::from(row) * u64::from(columns) + u64::from(column)) as usize]
    };
    let mut paths = Vec::new();
    for row in 0..rows {
        let top = view.y + row * span;
        let bottom = (top + span).min(view.y + view.height);
        for column in 0..columns {
            let left = view.x + column * span;
            let right = (left + span).min(view.x + view.width);
            match cell_at(column, row) {
                CoarseCell::Empty => {}
                CoarseCell::Mixed => {
                    let center_x = (left as f32 + right as f32) * 0.5;
                    let center_y = (top as f32 + bottom as f32) * 0.5;
                    let inset_x = (right - left) as f32 * 0.2;
                    let inset_y = (bottom - top) as f32 * 0.2;
                    paths.push(vec![
                        map_float_point(bounds, center_x, top as f32 + inset_y),
                        map_float_point(bounds, right as f32 - inset_x, center_y),
                        map_float_point(bounds, center_x, bottom as f32 - inset_y),
                        map_float_point(bounds, left as f32 + inset_x, center_y),
                        map_float_point(bounds, center_x, top as f32 + inset_y),
                    ]);
                }
                CoarseCell::Full => {
                    let top_boundary = row > 0 && cell_at(column, row - 1) != CoarseCell::Full
                        || row == 0 && view.y == 0;
                    let right_boundary = column + 1 < columns
                        && cell_at(column + 1, row) != CoarseCell::Full
                        || column + 1 == columns && view.x + view.width == bounds.2;
                    let bottom_boundary = row + 1 < rows
                        && cell_at(column, row + 1) != CoarseCell::Full
                        || row + 1 == rows && view.y + view.height == bounds.3;
                    let left_boundary = column > 0 && cell_at(column - 1, row) != CoarseCell::Full
                        || column == 0 && view.x == 0;
                    if top_boundary {
                        paths.push(vec![
                            map_point(bounds, (left, top)),
                            map_point(bounds, (right, top)),
                        ]);
                    }
                    if right_boundary {
                        paths.push(vec![
                            map_point(bounds, (right, top)),
                            map_point(bounds, (right, bottom)),
                        ]);
                    }
                    if bottom_boundary {
                        paths.push(vec![
                            map_point(bounds, (right, bottom)),
                            map_point(bounds, (left, bottom)),
                        ]);
                    }
                    if left_boundary {
                        paths.push(vec![
                            map_point(bounds, (left, bottom)),
                            map_point(bounds, (left, top)),
                        ]);
                    }
                }
            }
        }
    }
    debug_assert!(
        paths
            .iter()
            .map(|path| path.len().saturating_sub(1))
            .sum::<usize>()
            <= MAX_SELECTION_OUTLINE_EDGES
    );
    paths.into()
}

fn map_contours(
    contours: Vec<Vec<(u32, u32)>>,
    bounds: (u32, u32, u32, u32),
) -> Arc<[SelectionOutlinePath]> {
    contours
        .into_iter()
        .map(|contour| {
            contour
                .into_iter()
                .map(|point| map_point(bounds, point))
                .collect()
        })
        .collect()
}

fn map_point(bounds: (u32, u32, u32, u32), point: (u32, u32)) -> SelectionOutlinePoint {
    SelectionOutlinePoint::new(
        bounds.0 as f32 + point.0 as f32,
        bounds.1 as f32 + point.1 as f32,
    )
}

fn map_float_point(bounds: (u32, u32, u32, u32), x: f32, y: f32) -> SelectionOutlinePoint {
    SelectionOutlinePoint::new(bounds.0 as f32 + x, bounds.1 as f32 + y)
}

fn clipped_segment_parameter(
    start: SelectionOutlinePoint,
    end: SelectionOutlinePoint,
    clip: SelectionOutlineRect,
) -> Option<(f32, f32)> {
    let delta = SelectionOutlinePoint::new(end.x - start.x, end.y - start.y);
    let mut lower = 0.0_f32;
    let mut upper = 1.0_f32;
    for (origin, direction, minimum, maximum) in [
        (start.x, delta.x, clip.min.x, clip.max.x),
        (start.y, delta.y, clip.min.y, clip.max.y),
    ] {
        if direction.abs() <= f32::EPSILON {
            if origin < minimum || origin > maximum {
                return None;
            }
            continue;
        }
        let first = (minimum - origin) / direction;
        let second = (maximum - origin) / direction;
        let (near, far) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        lower = lower.max(near);
        upper = upper.min(far);
        if lower > upper {
            return None;
        }
    }
    Some((lower.clamp(0.0, 1.0), upper.clamp(0.0, 1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkerboard(edge: u32) -> Vec<u8> {
        let mut alpha = vec![0; (u64::from(edge) * u64::from(edge)) as usize];
        for y in 0..edge {
            for x in 0..edge {
                alpha[(u64::from(y) * u64::from(edge) + u64::from(x)) as usize] =
                    if (x + y) % 2 == 0 { 255 } else { 0 };
            }
        }
        alpha
    }

    #[test]
    fn pathological_checkerboard_stays_mixed_instead_of_becoming_a_rectangle() {
        let alpha = checkerboard(1_024);
        assert!(matches!(
            selection_mask_outline((0, 0, 1_024, 1_024), &alpha),
            SelectionMaskOutline::Complex
        ));
        let paths = complex_selection_mask_outline(
            (0, 0, 1_024, 1_024),
            &alpha,
            SelectionOutlineView {
                x: 0,
                y: 0,
                width: 1_024,
                height: 1_024,
            },
        );
        assert_eq!(paths.len(), MAX_COARSE_CELLS);
        assert!(paths.iter().all(|path| path.len() == 5));
        assert!(
            paths
                .iter()
                .any(|path| { path.iter().any(|point| point.x > 0.0 && point.x < 1_024.0) })
        );
    }

    #[test]
    fn irregular_antialiased_masks_produce_exact_closed_contours() {
        let alpha = [
            0, 0, 0, 0, 0, 0, 255, 160, 0, 0, 0, 255, 255, 0, 0, 0, 0, 0, 0, 255,
        ];
        let SelectionMaskOutline::Exact(paths) = selection_mask_outline((10, 20, 5, 4), &alpha)
        else {
            panic!("small mask should have exact contours");
        };
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|path| path.first() == path.last()));
        assert!(paths.iter().flatten().all(|point| {
            point.x >= 10.0 && point.x <= 15.0 && point.y >= 20.0 && point.y <= 24.0
        }));
    }

    #[test]
    fn malformed_alpha_returns_an_empty_exact_outline() {
        let SelectionMaskOutline::Exact(paths) = selection_mask_outline((0, 0, 4, 4), &[255; 15])
        else {
            panic!("malformed alpha must fail closed without complex work");
        };
        assert!(paths.is_empty());
    }

    #[test]
    fn zoomed_checkerboard_view_uses_exact_source_pixel_edges() {
        let alpha = checkerboard(1_024);
        let paths = complex_selection_mask_outline(
            (10, 20, 1_024, 1_024),
            &alpha,
            SelectionOutlineView {
                x: 100,
                y: 120,
                width: 16,
                height: 16,
            },
        );
        assert_eq!(paths.len(), 16 * 16 / 2 * 4);
        assert!(paths.iter().all(|path| path.len() == 2));
        assert!(paths.iter().flatten().all(|point| {
            point.x >= 110.0 && point.x <= 126.0 && point.y >= 140.0 && point.y <= 156.0
        }));
    }

    #[test]
    fn marching_ants_frame_clips_and_emits_dual_contrast_work() {
        let paths = vec![vec![
            SelectionOutlinePoint::new(-10_000.0, 50.0),
            SelectionOutlinePoint::new(10_000.0, 50.0),
        ]];
        let frame = marching_ants_frame(
            &paths,
            SelectionOutlineTransform {
                scale: 1.0,
                offset: SelectionOutlinePoint::new(0.0, 0.0),
            },
            SelectionOutlineRect::new(
                SelectionOutlinePoint::new(0.0, 0.0),
                SelectionOutlinePoint::new(100.0, 100.0),
            ),
            3.0,
            4.0,
        );
        assert_eq!(frame.contrast.len(), 1);
        assert!(!frame.light.is_empty());
        assert!(frame.contrast.iter().chain(&frame.light).all(|segment| {
            segment.start.x >= 0.0
                && segment.start.x <= 100.0
                && segment.end.x >= 0.0
                && segment.end.x <= 100.0
        }));
    }
}
