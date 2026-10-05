use anyhow::{Result, bail};

use crate::{BrushClip, BrushStroke, MAX_PAINT_REGION_PIXELS, Selection, Transform};

pub(crate) fn capture_selection_clip(
    selection: Option<&Selection>,
    stroke: &BrushStroke,
    viewport: (u32, u32),
    transform: Transform,
) -> Result<Option<BrushClip>> {
    let Some(selection) = selection else {
        return Ok(None);
    };
    let radius = stroke.style.size * 0.5 + 1.0;
    let min_x = stroke
        .samples
        .iter()
        .map(|sample| sample.x)
        .fold(f32::INFINITY, f32::min);
    let min_y = stroke
        .samples
        .iter()
        .map(|sample| sample.y)
        .fold(f32::INFINITY, f32::min);
    let max_x = stroke
        .samples
        .iter()
        .map(|sample| sample.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = stroke
        .samples
        .iter()
        .map(|sample| sample.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let x = (min_x - radius).floor().max(0.0) as u32;
    let y = (min_y - radius).floor().max(0.0) as u32;
    let right = (max_x + radius).ceil().min(viewport.0 as f32) as u32;
    let bottom = (max_y + radius).ceil().min(viewport.1 as f32) as u32;
    if let Selection::Rectangle {
        x: selection_x,
        y: selection_y,
        width: selection_width,
        height: selection_height,
    } = selection
        && rotation_is_zero(transform.rotation)
    {
        let (clip_x, clip_right) = axis_aligned_selection_range(
            *selection_x,
            *selection_width,
            transform.x,
            transform.scale_x,
            viewport.0,
        );
        let (clip_y, clip_bottom) = axis_aligned_selection_range(
            *selection_y,
            *selection_height,
            transform.y,
            transform.scale_y,
            viewport.1,
        );
        let x = x.max(clip_x);
        let y = y.max(clip_y);
        let right = right.min(clip_right);
        let bottom = bottom.min(clip_bottom);
        return Ok(Some(if right > x && bottom > y {
            BrushClip::Rectangle {
                x,
                y,
                width: right - x,
                height: bottom - y,
            }
        } else {
            empty_clip(viewport)
        }));
    }
    let Some((selection_x, selection_y, selection_right, selection_bottom)) =
        selection_local_bounds(selection, viewport, transform)
    else {
        return Ok(Some(empty_clip(viewport)));
    };
    let x = x.max(selection_x);
    let y = y.max(selection_y);
    let right = right.min(selection_right);
    let bottom = bottom.min(selection_bottom);
    if right <= x || bottom <= y {
        return Ok(Some(empty_clip(viewport)));
    }
    let width = right - x;
    let height = bottom - y;
    if u64::from(width) * u64::from(height) > MAX_PAINT_REGION_PIXELS {
        bail!("captured brush selection exceeds the bounded clip limit");
    }
    let mut alpha = Vec::with_capacity((width * height) as usize);
    for local_y in y..bottom {
        for local_x in x..right {
            let document_point = local_to_document_pixel_center(
                local_x as f32 + 0.5,
                local_y as f32 + 0.5,
                viewport,
                transform,
            );
            alpha.push(sample_selection_bilinear(selection, document_point));
        }
    }
    if let Some((rect_x, rect_y, rect_width, rect_height)) = opaque_rectangle(&alpha, width, height)
    {
        return Ok(Some(BrushClip::Rectangle {
            x: x + rect_x,
            y: y + rect_y,
            width: rect_width,
            height: rect_height,
        }));
    }
    Ok(Some(BrushClip::Alpha {
        x,
        y,
        width,
        height,
        alpha: alpha.into(),
    }))
}

fn rotation_is_zero(rotation: f32) -> bool {
    let normalized = rotation.rem_euclid(360.0);
    normalized.min(360.0 - normalized) <= f32::EPSILON
}

fn axis_aligned_selection_range(
    selection_start: u32,
    selection_length: u32,
    translation: f32,
    scale: f32,
    viewport_length: u32,
) -> (u32, u32) {
    let local_boundary =
        |document: f64| ((document - f64::from(translation)) / f64::from(scale) - 0.5).ceil();
    let start = local_boundary(f64::from(selection_start)).clamp(0.0, f64::from(viewport_length));
    let selection_end = u64::from(selection_start) + u64::from(selection_length);
    let end = local_boundary(selection_end as f64).clamp(0.0, f64::from(viewport_length));
    (start as u32, end as u32)
}

fn empty_clip(viewport: (u32, u32)) -> BrushClip {
    BrushClip::Alpha {
        x: 0,
        y: 0,
        width: 1.min(viewport.0),
        height: 1.min(viewport.1),
        alpha: vec![0].into(),
    }
}

fn selection_local_bounds(
    selection: &Selection,
    viewport: (u32, u32),
    transform: Transform,
) -> Option<(u32, u32, u32, u32)> {
    let (x, y, width, height) = selection.bounds();
    let padding = if matches!(selection, Selection::Rectangle { .. }) {
        0.0
    } else {
        1.0
    };
    let left = x as f32 - padding;
    let top = y as f32 - padding;
    let right = (u64::from(x) + u64::from(width)) as f32 + padding;
    let bottom = (u64::from(y) + u64::from(height)) as f32 + padding;
    let corners = [
        document_to_local(left, top, viewport, transform),
        document_to_local(right, top, viewport, transform),
        document_to_local(left, bottom, viewport, transform),
        document_to_local(right, bottom, viewport, transform),
    ];
    let min_x = corners
        .iter()
        .map(|point| point.0)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let min_y = corners
        .iter()
        .map(|point| point.1)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let max_x = corners
        .iter()
        .map(|point| point.0)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(viewport.0 as f32) as u32;
    let max_y = corners
        .iter()
        .map(|point| point.1)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(viewport.1 as f32) as u32;
    (max_x > min_x && max_y > min_y).then_some((min_x, min_y, max_x, max_y))
}

fn document_to_local(x: f32, y: f32, viewport: (u32, u32), transform: Transform) -> (f32, f32) {
    let center = (
        viewport.0 as f32 * transform.scale_x * 0.5,
        viewport.1 as f32 * transform.scale_y * 0.5,
    );
    let dx = x - transform.x - center.0;
    let dy = y - transform.y - center.1;
    let radians = -transform.rotation.to_radians();
    let (sin, cos) = radians.sin_cos();
    (
        (dx * cos - dy * sin + center.0) / transform.scale_x,
        (dx * sin + dy * cos + center.1) / transform.scale_y,
    )
}

fn opaque_rectangle(alpha: &[u8], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let mut left = width;
    let mut top = height;
    let mut right = 0;
    let mut bottom = 0;
    let mut found = false;
    for y in 0..height {
        for x in 0..width {
            let value = alpha[(y * width + x) as usize];
            if value != 0 && value != 255 {
                return None;
            }
            if value == 255 {
                found = true;
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if !found {
        return None;
    }
    for y in top..bottom {
        for x in left..right {
            if alpha[(y * width + x) as usize] != 255 {
                return None;
            }
        }
    }
    Some((left, top, right - left, bottom - top))
}

fn local_to_document_pixel_center(
    x: f32,
    y: f32,
    viewport: (u32, u32),
    transform: Transform,
) -> (f32, f32) {
    let scaled = (x * transform.scale_x, y * transform.scale_y);
    let center = (
        viewport.0 as f32 * transform.scale_x * 0.5,
        viewport.1 as f32 * transform.scale_y * 0.5,
    );
    let radians = transform.rotation.to_radians();
    let (sin, cos) = radians.sin_cos();
    let dx = scaled.0 - center.0;
    let dy = scaled.1 - center.1;
    (
        transform.x + center.0 + dx * cos - dy * sin,
        transform.y + center.1 + dx * sin + dy * cos,
    )
}

fn sample_selection_bilinear(selection: &Selection, point: (f32, f32)) -> u8 {
    let (x, y, width, height) = selection.bounds();
    if matches!(selection, Selection::Rectangle { .. }) {
        return if point.0 >= x as f32
            && point.1 >= y as f32
            && point.0 < (x + width) as f32
            && point.1 < (y + height) as f32
        {
            255
        } else {
            0
        };
    }
    let Some(alpha) = selection.alpha() else {
        return 0;
    };
    let sample_x = point.0 - x as f32 - 0.5;
    let sample_y = point.1 - y as f32 - 0.5;
    let left = sample_x.floor() as i64;
    let top = sample_y.floor() as i64;
    let fraction_x = sample_x - left as f32;
    let fraction_y = sample_y - top as f32;
    let at = |px: i64, py: i64| -> f32 {
        if px < 0 || py < 0 || px >= i64::from(width) || py >= i64::from(height) {
            0.0
        } else {
            f32::from(alpha[(py as u32 * width + px as u32) as usize])
        }
    };
    let top_value = at(left, top) * (1.0 - fraction_x) + at(left + 1, top) * fraction_x;
    let bottom_value = at(left, top + 1) * (1.0 - fraction_x) + at(left + 1, top + 1) * fraction_x;
    (top_value * (1.0 - fraction_y) + bottom_value * fraction_y)
        .round()
        .clamp(0.0, 255.0) as u8
}
