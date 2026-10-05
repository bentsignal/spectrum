use anyhow::{Result, bail};
use serde::{Deserialize, Deserializer, Serialize, de::SeqAccess};

use crate::{MAX_COLOR_SELECTION_PIXELS, Selection};

pub const LASSO_SUBPIXEL_SCALE: i64 = 256;
pub const MAX_LASSO_INPUT_POINTS: usize = 8_192;
pub const MAX_LASSO_VERTICES: usize = 1_024;
pub const MAX_LASSO_RASTER_EDGE_TESTS: u64 = 256 * 1024 * 1024;
const SIMPLIFY_EPSILON: i64 = LASSO_SUBPIXEL_SCALE / 4;
const RDP_WORK_BUDGET: usize = MAX_LASSO_INPUT_POINTS * 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LassoPoint {
    /// Document-space x coordinate in 1/256-pixel fixed-point units.
    pub x: i64,
    /// Document-space y coordinate in 1/256-pixel fixed-point units.
    pub y: i64,
}

impl LassoPoint {
    pub fn from_canvas(x: f32, y: f32) -> Result<Self> {
        if !x.is_finite() || !y.is_finite() {
            bail!("lasso coordinates must be finite");
        }
        let quantize = |value: f32| (f64::from(value) * LASSO_SUBPIXEL_SCALE as f64).round();
        let x = quantize(x);
        let y = quantize(y);
        if x < i64::MIN as f64 || x > i64::MAX as f64 || y < i64::MIN as f64 || y > i64::MAX as f64
        {
            bail!("lasso coordinate exceeds the fixed-point range");
        }
        Ok(Self {
            x: x as i64,
            y: y as i64,
        })
    }

    fn clamped(self, canvas_width: u32, canvas_height: u32) -> Self {
        Self {
            x: self
                .x
                .clamp(0, i64::from(canvas_width) * LASSO_SUBPIXEL_SCALE),
            y: self
                .y
                .clamp(0, i64::from(canvas_height) * LASSO_SUBPIXEL_SCALE),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct LassoPath(Vec<LassoPoint>);

impl LassoPath {
    pub fn new(points: Vec<LassoPoint>) -> Result<Self> {
        if points.len() > MAX_LASSO_INPUT_POINTS {
            bail!("lasso path exceeds the {MAX_LASSO_INPUT_POINTS}-point limit");
        }
        Ok(Self(points))
    }

    pub fn points(&self) -> &[LassoPoint] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for LassoPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PathVisitor;

        impl<'de> serde::de::Visitor<'de> for PathVisitor {
            type Value = LassoPath;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(
                    formatter,
                    "at most {MAX_LASSO_INPUT_POINTS} fixed-point lasso points"
                )
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut points = Vec::with_capacity(
                    sequence
                        .size_hint()
                        .unwrap_or_default()
                        .min(MAX_LASSO_INPUT_POINTS),
                );
                while let Some(point) = sequence.next_element()? {
                    if points.len() == MAX_LASSO_INPUT_POINTS {
                        return Err(serde::de::Error::custom(format!(
                            "lasso path exceeds the {MAX_LASSO_INPUT_POINTS}-point limit"
                        )));
                    }
                    points.push(point);
                }
                Ok(LassoPath(points))
            }
        }

        deserializer.deserialize_seq(PathVisitor)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionCombineMode {
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}

pub fn lasso_selection(
    canvas_width: u32,
    canvas_height: u32,
    path: &LassoPath,
    antialias: bool,
) -> Result<Selection> {
    if canvas_width == 0 || canvas_height == 0 {
        bail!("lasso requires a nonempty canvas");
    }
    let points = simplify_closed_ring(path, canvas_width, canvas_height)?;
    let min_x = points.iter().map(|point| point.x).min().unwrap_or_default();
    let min_y = points.iter().map(|point| point.y).min().unwrap_or_default();
    let max_x = points.iter().map(|point| point.x).max().unwrap_or_default();
    let max_y = points.iter().map(|point| point.y).max().unwrap_or_default();
    let left = (min_x / LASSO_SUBPIXEL_SCALE) as u32;
    let top = (min_y / LASSO_SUBPIXEL_SCALE) as u32;
    let right = ((max_x + LASSO_SUBPIXEL_SCALE - 1) / LASSO_SUBPIXEL_SCALE)
        .min(i64::from(canvas_width)) as u32;
    let bottom = ((max_y + LASSO_SUBPIXEL_SCALE - 1) / LASSO_SUBPIXEL_SCALE)
        .min(i64::from(canvas_height)) as u32;
    if right <= left || bottom <= top {
        bail!("lasso path does not cover a document pixel");
    }
    let width = right - left;
    let height = bottom - top;
    check_mask_pixels(width, height)?;
    let offsets: &[i64] = if antialias {
        &[32, 96, 160, 224]
    } else {
        &[128]
    };
    let sample_count = offsets.len() * offsets.len();
    let raster_work = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|work| work.checked_mul(sample_count as u64))
        .and_then(|work| work.checked_mul(points.len() as u64))
        .ok_or_else(|| anyhow::anyhow!("lasso raster work estimate overflow"))?;
    if raster_work > MAX_LASSO_RASTER_EDGE_TESTS {
        bail!(
            "lasso raster requires {raster_work} edge tests; simplify or tighten the path below the {MAX_LASSO_RASTER_EDGE_TESTS}-test limit"
        );
    }
    let mut alpha = vec![0_u8; mask_len(width, height)?];
    for y in 0..height {
        for x in 0..width {
            let mut covered = 0_usize;
            for offset_y in offsets {
                for offset_x in offsets {
                    let sample_x = i64::from(left + x) * LASSO_SUBPIXEL_SCALE + offset_x;
                    let sample_y = i64::from(top + y) * LASSO_SUBPIXEL_SCALE + offset_y;
                    covered += usize::from(point_in_even_odd_ring(&points, sample_x, sample_y));
                }
            }
            alpha[(u64::from(y) * u64::from(width) + u64::from(x)) as usize] =
                ((covered * 255 + sample_count / 2) / sample_count) as u8;
        }
    }
    canonical_selection(left, top, width, height, alpha)?
        .ok_or_else(|| anyhow::anyhow!("lasso path produced an empty pixel selection"))
}

pub fn apply_lasso_selection(
    current: Option<&Selection>,
    canvas_width: u32,
    canvas_height: u32,
    path: &LassoPath,
    mode: SelectionCombineMode,
    antialias: bool,
) -> Result<Option<Selection>> {
    let incoming = lasso_selection(canvas_width, canvas_height, path, antialias)?;
    combine_selections(current, &incoming, mode)
}

pub fn combine_selections(
    current: Option<&Selection>,
    incoming: &Selection,
    mode: SelectionCombineMode,
) -> Result<Option<Selection>> {
    let Some(current) = current else {
        return Ok(match mode {
            SelectionCombineMode::Replace | SelectionCombineMode::Add => Some(incoming.clone()),
            SelectionCombineMode::Subtract | SelectionCombineMode::Intersect => None,
        });
    };
    if mode == SelectionCombineMode::Replace {
        return Ok(Some(incoming.clone()));
    }
    let current_bounds = current.bounds();
    let incoming_bounds = incoming.bounds();
    let overlap = intersect_bounds(current_bounds, incoming_bounds);

    match mode {
        SelectionCombineMode::Replace => unreachable!(),
        SelectionCombineMode::Add => {
            if opaque_rectangle_contains(current, incoming_bounds) {
                return Ok(Some(current.clone()));
            }
            if opaque_rectangle_contains(incoming, current_bounds) {
                return Ok(Some(incoming.clone()));
            }
            if matches!(
                (current, incoming),
                (Selection::Rectangle { .. }, Selection::Rectangle { .. })
            ) && let Some(union) = rectangular_union(current_bounds, incoming_bounds)
            {
                return Ok(Some(Selection::rectangle(
                    union.0, union.1, union.2, union.3,
                )));
            }
        }
        SelectionCombineMode::Subtract => {
            if overlap.is_none() {
                return Ok(Some(current.clone()));
            }
            if opaque_rectangle_contains(incoming, current_bounds) {
                return Ok(None);
            }
            if matches!(
                (current, incoming),
                (Selection::Rectangle { .. }, Selection::Rectangle { .. })
            ) && let Some(remainder) = rectangular_difference(current_bounds, incoming_bounds)
            {
                return Ok(remainder
                    .map(|bounds| Selection::rectangle(bounds.0, bounds.1, bounds.2, bounds.3)));
            }
        }
        SelectionCombineMode::Intersect => {
            let Some(overlap) = overlap else {
                return Ok(None);
            };
            if matches!(
                (current, incoming),
                (Selection::Rectangle { .. }, Selection::Rectangle { .. })
            ) {
                return Ok(Some(Selection::rectangle(
                    overlap.0, overlap.1, overlap.2, overlap.3,
                )));
            }
            if opaque_rectangle_contains(current, incoming_bounds) {
                return Ok(Some(incoming.clone()));
            }
            if opaque_rectangle_contains(incoming, current_bounds) {
                return Ok(Some(current.clone()));
            }
        }
    }

    let bounds = match mode {
        SelectionCombineMode::Replace => unreachable!(),
        SelectionCombineMode::Add => union_bounds(current_bounds, incoming_bounds)?,
        SelectionCombineMode::Subtract => current_bounds,
        SelectionCombineMode::Intersect => overlap.expect("intersection checked above"),
    };
    check_mask_pixels(bounds.2, bounds.3)?;
    let mut alpha = Vec::with_capacity(mask_len(bounds.2, bounds.3)?);
    for y in bounds.1..bounds.1 + bounds.3 {
        for x in bounds.0..bounds.0 + bounds.2 {
            let left = selection_alpha_at(current, x, y);
            let right = selection_alpha_at(incoming, x, y);
            let intersection = multiply_alpha(left, right);
            alpha.push(match mode {
                SelectionCombineMode::Replace => unreachable!(),
                SelectionCombineMode::Add => {
                    (u16::from(left) + u16::from(right) - u16::from(intersection)) as u8
                }
                SelectionCombineMode::Subtract => multiply_alpha(left, 255 - right),
                SelectionCombineMode::Intersect => intersection,
            });
        }
    }
    canonical_selection(bounds.0, bounds.1, bounds.2, bounds.3, alpha)
}

fn simplify_closed_ring(
    path: &LassoPath,
    canvas_width: u32,
    canvas_height: u32,
) -> Result<Vec<LassoPoint>> {
    if path.points().len() < 3 {
        bail!("lasso path requires at least three points");
    }
    let mut ring = Vec::with_capacity(path.points().len());
    for point in path
        .points()
        .iter()
        .map(|point| point.clamped(canvas_width, canvas_height))
    {
        if ring.last() != Some(&point) {
            ring.push(point);
        }
    }
    if ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    if ring.len() < 3 {
        bail!("lasso path requires three distinct consecutive points");
    }

    let anchor = ring
        .iter()
        .enumerate()
        .min_by_key(|(index, point)| (point.x, point.y, *index))
        .map(|(index, _)| index)
        .unwrap_or_default();
    let opposite = ring
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != anchor)
        .max_by_key(|(index, point)| (distance_squared(ring[anchor], **point), usize::MAX - *index))
        .map(|(index, _)| index)
        .unwrap_or(anchor);
    let first = ring_chain(&ring, anchor, opposite);
    let second = ring_chain(&ring, opposite, anchor);
    let mut work = 0_usize;
    let Some(first) = simplify_open_chain(&first, &mut work) else {
        return Ok(evenly_sample_ring(&ring, MAX_LASSO_VERTICES));
    };
    let Some(second) = simplify_open_chain(&second, &mut work) else {
        return Ok(evenly_sample_ring(&ring, MAX_LASSO_VERTICES));
    };
    let mut simplified = first;
    simplified.extend(second.into_iter().skip(1).rev().skip(1).rev());
    simplified.dedup();
    if simplified.len() > MAX_LASSO_VERTICES {
        simplified = evenly_sample_ring(&simplified, MAX_LASSO_VERTICES);
    }
    if simplified.len() < 3 {
        bail!("lasso simplification produced fewer than three vertices");
    }
    Ok(simplified)
}

fn ring_chain(ring: &[LassoPoint], start: usize, end: usize) -> Vec<LassoPoint> {
    let mut chain = vec![ring[start]];
    let mut index = start;
    while index != end {
        index = (index + 1) % ring.len();
        chain.push(ring[index]);
    }
    chain
}

fn simplify_open_chain(points: &[LassoPoint], work: &mut usize) -> Option<Vec<LassoPoint>> {
    if points.len() <= 2 {
        return Some(points.to_vec());
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0_usize, points.len() - 1)];
    while let Some((start, end)) = stack.pop() {
        let mut farthest = None;
        let mut farthest_score = 0_u128;
        for index in start + 1..end {
            *work += 1;
            if *work > RDP_WORK_BUDGET {
                return None;
            }
            let score = segment_distance_score(points[index], points[start], points[end]);
            if score > farthest_score {
                farthest_score = score;
                farthest = Some(index);
            }
        }
        if farthest_score > SIMPLIFY_EPSILON as u128
            && let Some(index) = farthest
        {
            keep[index] = true;
            stack.push((index, end));
            stack.push((start, index));
        }
    }
    Some(
        points
            .iter()
            .zip(keep)
            .filter_map(|(point, keep)| keep.then_some(*point))
            .collect(),
    )
}

fn segment_distance_score(point: LassoPoint, start: LassoPoint, end: LassoPoint) -> u128 {
    let dx = i128::from(end.x - start.x);
    let dy = i128::from(end.y - start.y);
    let px = i128::from(point.x - start.x);
    let py = i128::from(point.y - start.y);
    let length_squared = (dx * dx + dy * dy) as u128;
    if length_squared == 0 {
        return integer_sqrt(distance_squared(point, start));
    }
    let dot = px * dx + py * dy;
    if dot <= 0 {
        return integer_sqrt(distance_squared(point, start));
    }
    if dot as u128 >= length_squared {
        return integer_sqrt(distance_squared(point, end));
    }
    let cross = (px * dy - py * dx).unsigned_abs();
    cross / integer_sqrt(length_squared).max(1)
}

fn evenly_sample_ring(points: &[LassoPoint], limit: usize) -> Vec<LassoPoint> {
    if points.len() <= limit {
        return points.to_vec();
    }
    (0..limit)
        .map(|index| points[index * points.len() / limit])
        .collect()
}

fn integer_sqrt(value: u128) -> u128 {
    if value < 2 {
        return value;
    }
    let mut low = 1_u128;
    let mut high = value.min(u128::from(u64::MAX)) + 1;
    while low + 1 < high {
        let middle = low + (high - low) / 2;
        if middle <= value / middle {
            low = middle;
        } else {
            high = middle;
        }
    }
    low
}

fn distance_squared(left: LassoPoint, right: LassoPoint) -> u128 {
    let dx = i128::from(left.x - right.x);
    let dy = i128::from(left.y - right.y);
    (dx * dx + dy * dy) as u128
}

#[cfg(test)]
fn signed_double_area(points: &[LassoPoint]) -> i128 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| {
            i128::from(left.x) * i128::from(right.y) - i128::from(right.x) * i128::from(left.y)
        })
        .sum()
}

fn point_in_even_odd_ring(points: &[LassoPoint], sample_x: i64, sample_y: i64) -> bool {
    let mut inside = false;
    for (left, right) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        if (left.y > sample_y) == (right.y > sample_y) {
            continue;
        }
        let dy = i128::from(right.y - left.y);
        let lhs = i128::from(sample_x - left.x) * dy;
        let rhs = i128::from(sample_y - left.y) * i128::from(right.x - left.x);
        if (dy > 0 && lhs < rhs) || (dy < 0 && lhs > rhs) {
            inside = !inside;
        }
    }
    inside
}

fn multiply_alpha(left: u8, right: u8) -> u8 {
    ((u16::from(left) * u16::from(right) + 127) / 255) as u8
}

fn selection_alpha_at(selection: &Selection, x: u32, y: u32) -> u8 {
    let (origin_x, origin_y, width, height) = selection.bounds();
    if x < origin_x || y < origin_y || x >= origin_x + width || y >= origin_y + height {
        return 0;
    }
    selection.alpha().map_or(255, |alpha| {
        let local_x = x - origin_x;
        let local_y = y - origin_y;
        alpha[(u64::from(local_y) * u64::from(width) + u64::from(local_x)) as usize]
    })
}

fn opaque_rectangle_contains(selection: &Selection, bounds: (u32, u32, u32, u32)) -> bool {
    matches!(selection, Selection::Rectangle { .. }) && bounds_contains(selection.bounds(), bounds)
}

fn bounds_contains(outer: (u32, u32, u32, u32), inner: (u32, u32, u32, u32)) -> bool {
    inner.0 >= outer.0
        && inner.1 >= outer.1
        && inner.0 + inner.2 <= outer.0 + outer.2
        && inner.1 + inner.3 <= outer.1 + outer.3
}

fn intersect_bounds(
    left: (u32, u32, u32, u32),
    right: (u32, u32, u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    let x = left.0.max(right.0);
    let y = left.1.max(right.1);
    let far_x = (left.0 + left.2).min(right.0 + right.2);
    let far_y = (left.1 + left.3).min(right.1 + right.3);
    (far_x > x && far_y > y).then(|| (x, y, far_x - x, far_y - y))
}

fn union_bounds(
    left: (u32, u32, u32, u32),
    right: (u32, u32, u32, u32),
) -> Result<(u32, u32, u32, u32)> {
    let x = left.0.min(right.0);
    let y = left.1.min(right.1);
    let far_x = left
        .0
        .checked_add(left.2)
        .and_then(|far| far.max(right.0.checked_add(right.2)?).checked_sub(x))
        .ok_or_else(|| anyhow::anyhow!("selection union bounds overflow"))?;
    let far_y = left
        .1
        .checked_add(left.3)
        .and_then(|far| far.max(right.1.checked_add(right.3)?).checked_sub(y))
        .ok_or_else(|| anyhow::anyhow!("selection union bounds overflow"))?;
    Ok((x, y, far_x, far_y))
}

fn rectangular_union(
    left: (u32, u32, u32, u32),
    right: (u32, u32, u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    let union = union_bounds(left, right).ok()?;
    let overlap_area = intersect_bounds(left, right)
        .map(|bounds| u64::from(bounds.2) * u64::from(bounds.3))
        .unwrap_or_default();
    let covered_area = u64::from(left.2) * u64::from(left.3)
        + u64::from(right.2) * u64::from(right.3)
        - overlap_area;
    (u64::from(union.2) * u64::from(union.3) == covered_area).then_some(union)
}

fn rectangular_difference(
    current: (u32, u32, u32, u32),
    incoming: (u32, u32, u32, u32),
) -> Option<Option<(u32, u32, u32, u32)>> {
    let overlap = intersect_bounds(current, incoming)?;
    if overlap == current {
        return Some(None);
    }
    let current_right = current.0 + current.2;
    let current_bottom = current.1 + current.3;
    let overlap_right = overlap.0 + overlap.2;
    let overlap_bottom = overlap.1 + overlap.3;
    let remainder = if overlap.0 == current.0 && overlap.2 == current.2 && overlap.1 == current.1 {
        (
            current.0,
            overlap_bottom,
            current.2,
            current_bottom - overlap_bottom,
        )
    } else if overlap.0 == current.0 && overlap.2 == current.2 && overlap_bottom == current_bottom {
        (current.0, current.1, current.2, overlap.1 - current.1)
    } else if overlap.1 == current.1 && overlap.3 == current.3 && overlap.0 == current.0 {
        (
            overlap_right,
            current.1,
            current_right - overlap_right,
            current.3,
        )
    } else if overlap.1 == current.1 && overlap.3 == current.3 && overlap_right == current_right {
        (current.0, current.1, overlap.0 - current.0, current.3)
    } else {
        return None;
    };
    Some((remainder.2 != 0 && remainder.3 != 0).then_some(remainder))
}

fn canonical_selection(
    origin_x: u32,
    origin_y: u32,
    width: u32,
    height: u32,
    alpha: Vec<u8>,
) -> Result<Option<Selection>> {
    check_mask_pixels(width, height)?;
    if alpha.len() != mask_len(width, height)? {
        bail!("selection alpha length does not match its bounds");
    }
    let mut left = width;
    let mut top = height;
    let mut right = 0_u32;
    let mut bottom = 0_u32;
    for y in 0..height {
        for x in 0..width {
            if alpha[(u64::from(y) * u64::from(width) + u64::from(x)) as usize] == 0 {
                continue;
            }
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    if right <= left || bottom <= top {
        return Ok(None);
    }
    let output_width = right - left;
    let output_height = bottom - top;
    let mut trimmed = Vec::with_capacity(mask_len(output_width, output_height)?);
    for y in top..bottom {
        let start = (u64::from(y) * u64::from(width) + u64::from(left)) as usize;
        trimmed.extend_from_slice(&alpha[start..start + output_width as usize]);
    }
    if trimmed.iter().all(|value| *value == 255) {
        return Ok(Some(Selection::rectangle(
            origin_x + left,
            origin_y + top,
            output_width,
            output_height,
        )));
    }
    Ok(Some(Selection::color_mask(
        origin_x + left,
        origin_y + top,
        output_width,
        output_height,
        trimmed,
    )))
}

fn check_mask_pixels(width: u32, height: u32) -> Result<()> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_COLOR_SELECTION_PIXELS {
        bail!("lasso selection exceeds the {MAX_COLOR_SELECTION_PIXELS}-pixel mask limit");
    }
    Ok(())
}

fn mask_len(width: u32, height: u32) -> Result<usize> {
    Ok(usize::try_from(u64::from(width) * u64::from(height))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, y: f32) -> LassoPoint {
        LassoPoint::from_canvas(x, y).unwrap()
    }

    fn path(points: &[(f32, f32)]) -> LassoPath {
        LassoPath::new(points.iter().map(|(x, y)| point(*x, *y)).collect()).unwrap()
    }

    #[test]
    fn fixed_point_polygon_fill_is_deterministic_and_antialiased() {
        let triangle = path(&[(1.0, 1.0), (5.0, 1.0), (1.0, 5.0)]);
        let hard = lasso_selection(8, 8, &triangle, false).unwrap();
        assert_eq!(hard.bounds(), (1, 1, 3, 3));
        assert!(hard.alpha().is_some());
        let soft = lasso_selection(8, 8, &triangle, true).unwrap();
        assert_eq!(soft.bounds(), (1, 1, 4, 4));
        assert!(
            soft.alpha()
                .unwrap()
                .iter()
                .any(|alpha| (1..255).contains(alpha))
        );
        assert_eq!(
            serde_json::to_vec(&soft).unwrap(),
            serde_json::to_vec(&lasso_selection(8, 8, &triangle, true).unwrap()).unwrap()
        );
    }

    #[test]
    fn self_intersections_use_even_odd_fill() {
        let bow = path(&[(1.0, 1.0), (5.0, 5.0), (5.0, 1.0), (1.0, 5.0)]);
        let selection = lasso_selection(8, 8, &bow, false).unwrap();
        assert_eq!(selection.bounds(), (1, 1, 4, 4));
        let ring = simplify_closed_ring(&bow, 8, 8).unwrap();
        assert!(point_in_even_odd_ring(&ring, 2 * 256, 704));
        assert!(!point_in_even_odd_ring(&ring, 3 * 256, 704));
    }

    #[test]
    fn points_are_clamped_and_degenerate_rings_fail() {
        let outside = path(&[(-20.0, -20.0), (4.0, 0.0), (0.0, 4.0)]);
        assert_eq!(
            lasso_selection(8, 8, &outside, false).unwrap().bounds(),
            (0, 0, 3, 3)
        );
        let line = path(&[(0.0, 0.0), (2.0, 2.0), (4.0, 4.0)]);
        assert!(lasso_selection(8, 8, &line, true).is_err());
    }

    #[test]
    fn closed_ring_simplification_is_bounded_and_preserves_area() {
        let mut points = Vec::new();
        for x in 0..2_000 {
            points.push(point(x as f32 / 10.0, 0.0));
        }
        for y in 1..2_000 {
            points.push(point(199.9, y as f32 / 10.0));
        }
        for x in (0..2_000).rev() {
            points.push(point(x as f32 / 10.0, 199.9));
        }
        for y in (1..2_000).rev() {
            points.push(point(0.0, y as f32 / 10.0));
        }
        let path = LassoPath::new(points).unwrap();
        let simplified = simplify_closed_ring(&path, 512, 512).unwrap();
        assert!(simplified.len() <= MAX_LASSO_VERTICES);
        assert!(signed_double_area(&simplified) != 0);
    }

    #[test]
    fn exact_alpha_algebra_and_empty_results_are_locked() {
        let current = Selection::color_mask(0, 0, 1, 1, vec![128]);
        let incoming = Selection::color_mask(0, 0, 1, 1, vec![128]);
        let add = combine_selections(Some(&current), &incoming, SelectionCombineMode::Add)
            .unwrap()
            .unwrap();
        assert_eq!(add.alpha(), Some([192].as_slice()));
        let subtract =
            combine_selections(Some(&current), &incoming, SelectionCombineMode::Subtract)
                .unwrap()
                .unwrap();
        assert_eq!(subtract.alpha(), Some([64].as_slice()));
        let intersect =
            combine_selections(Some(&current), &incoming, SelectionCombineMode::Intersect)
                .unwrap()
                .unwrap();
        assert_eq!(intersect.alpha(), Some([64].as_slice()));
        assert_eq!(
            combine_selections(None, &incoming, SelectionCombineMode::Subtract).unwrap(),
            None
        );
    }

    #[test]
    fn rectangle_fast_paths_keep_large_canvas_operations_compact() {
        let select_all = Selection::rectangle(0, 0, 16_384, 16_384);
        let tiny = Selection::color_mask(100, 100, 2, 2, vec![255, 128, 128, 255]);
        assert_eq!(
            combine_selections(Some(&select_all), &tiny, SelectionCombineMode::Add).unwrap(),
            Some(select_all.clone())
        );
        assert_eq!(
            combine_selections(Some(&select_all), &tiny, SelectionCombineMode::Intersect).unwrap(),
            Some(tiny.clone())
        );
        let disjoint = Selection::rectangle(20_000, 20_000, 2, 2);
        assert_eq!(
            combine_selections(Some(&select_all), &disjoint, SelectionCombineMode::Subtract)
                .unwrap(),
            Some(select_all)
        );
    }

    #[test]
    fn huge_unrepresentable_union_fails_before_allocation() {
        let left = Selection::rectangle(0, 0, 1, 1);
        let right = Selection::rectangle(16_383, 16_383, 1, 1);
        assert!(combine_selections(Some(&left), &right, SelectionCombineMode::Add).is_err());
    }

    #[test]
    fn rectangle_union_and_edge_subtraction_stay_zero_byte() {
        let left = Selection::rectangle(0, 0, 8_192, 16_384);
        let right = Selection::rectangle(8_192, 0, 8_192, 16_384);
        assert_eq!(
            combine_selections(Some(&left), &right, SelectionCombineMode::Add).unwrap(),
            Some(Selection::rectangle(0, 0, 16_384, 16_384))
        );
        let strip = Selection::rectangle(0, 0, 1_024, 16_384);
        assert_eq!(
            combine_selections(
                Some(&Selection::rectangle(0, 0, 16_384, 16_384)),
                &strip,
                SelectionCombineMode::Subtract,
            )
            .unwrap(),
            Some(Selection::rectangle(1_024, 0, 15_360, 16_384))
        );
    }

    #[test]
    fn serialized_paths_reject_more_than_the_point_cap() {
        let json = format!(
            "[{}]",
            std::iter::repeat_n("{\"x\":0,\"y\":0}", MAX_LASSO_INPUT_POINTS + 1)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(serde_json::from_str::<LassoPath>(&json).is_err());
    }

    #[test]
    fn raster_work_limit_rejects_large_polygons_before_alpha_allocation() {
        let full = path(&[(0.0, 0.0), (4_096.0, 0.0), (0.0, 4_096.0)]);
        let error = lasso_selection(4_096, 4_096, &full, true).unwrap_err();
        assert!(error.to_string().contains("edge tests"));
    }
}
