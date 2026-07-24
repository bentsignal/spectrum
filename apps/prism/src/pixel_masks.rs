use anyhow::{Context, Result, bail};
use image::{DynamicImage, Rgba, RgbaImage};

use crate::{Document, Layer, LayerKind, PixelMask, Selection, Transform, render_layer_preview};

pub(crate) fn delete_selected_raster_pixels(document: &mut Document, id: u64) -> Result<()> {
    let selection = document
        .selection
        .clone()
        .context("create a pixel selection before deleting pixels")?;
    let layer = document.layer(id)?.clone();
    if layer.locked {
        bail!("layer {id} is locked");
    }
    let LayerKind::Raster { path, .. } = &layer.kind else {
        bail!("layer {id} is not a raster image layer");
    };
    let source_dimensions = image::image_dimensions(path)
        .with_context(|| format!("could not inspect raster layer {}", path.display()))?;
    let adjusted_dimensions = spectrum_imaging::adjusted_image_dimensions(
        source_dimensions.0,
        source_dimensions.1,
        &layer.adjustments,
    )
    .context("raster layer has invalid adjusted dimensions")?;
    let source_pixels = u64::from(source_dimensions.0) * u64::from(source_dimensions.1);
    let adjusted_pixels = u64::from(adjusted_dimensions.0) * u64::from(adjusted_dimensions.1);
    if source_pixels > crate::MAX_COLOR_SELECTION_PIXELS
        || adjusted_pixels > crate::MAX_COLOR_SELECTION_PIXELS
    {
        bail!(
            "raster pixel deletion is bounded to {} source and adjusted pixels",
            crate::MAX_COLOR_SELECTION_PIXELS
        );
    }
    let preview = render_layer_preview(&layer, None)?.into_rgba8();
    if preview.dimensions() != adjusted_dimensions {
        bail!("raster preview dimensions do not match its adjusted source");
    }
    let mut deletion = vec![0_u8; usize::try_from(source_pixels)?];
    let mapper = spectrum_imaging::AdjustedPixelSourceMapper::new(
        source_dimensions.0,
        source_dimensions.1,
        &layer.adjustments,
    )
    .context("raster layer has invalid source dimensions")?;
    if mapper.adjusted_dimensions() != adjusted_dimensions {
        bail!("raster source mapping dimensions do not match its adjusted preview");
    }
    accumulate_selection_deletion(
        &selection,
        &layer,
        source_dimensions,
        adjusted_dimensions,
        mapper,
        &preview,
        &mut deletion,
    );
    if deletion.iter().all(|coverage| *coverage == 0) {
        bail!("selection does not cover any visible pixels on raster layer {id}");
    }

    let existing = layer.pixel_mask.as_ref();
    let mut alpha = existing
        .map(|mask| mask.alpha.as_ref().to_vec())
        .unwrap_or_else(|| vec![255; deletion.len()]);
    for (alpha, deletion) in alpha.iter_mut().zip(deletion) {
        *alpha = multiply_alpha(*alpha, 255 - deletion);
    }
    if existing.is_some_and(|mask| mask.alpha.as_ref() == alpha) {
        bail!("selected raster pixels are already deleted");
    }
    document.validate_projected_inline_mask_budget(
        document.selection.as_ref(),
        if existing.is_none() { alpha.len() } else { 0 },
    )?;
    document.layer_mut(id)?.pixel_mask = Some(PixelMask::new(
        source_dimensions.0,
        source_dimensions.1,
        alpha,
    ));
    document.validate_inline_mask_budget()?;
    Ok(())
}

fn accumulate_selection_deletion(
    selection: &Selection,
    layer: &Layer,
    source_dimensions: (u32, u32),
    adjusted_dimensions: (u32, u32),
    mapper: spectrum_imaging::AdjustedPixelSourceMapper,
    preview: &RgbaImage,
    deletion: &mut [u8],
) {
    if let Selection::Rectangle {
        x,
        y,
        width,
        height,
    } = selection
    {
        accumulate_rectangle_deletion(
            (*x, *y, *width, *height),
            layer,
            source_dimensions,
            adjusted_dimensions,
            mapper,
            preview,
            deletion,
        );
        return;
    }
    let (selection_x, selection_y, selection_width, selection_height) = selection.bounds();
    for canvas_y in selection_y..selection_y.saturating_add(selection_height) {
        for canvas_x in selection_x..selection_x.saturating_add(selection_width) {
            let coverage = selection.alpha_at(canvas_x, canvas_y);
            if coverage == 0 {
                continue;
            }
            let Some((x_range, y_range)) = raster_source_sample_ranges(
                adjusted_dimensions,
                layer.transform,
                canvas_x,
                canvas_y,
            ) else {
                continue;
            };
            for adjusted_y in y_range.clone() {
                for adjusted_x in x_range.clone() {
                    if preview.get_pixel(adjusted_x, adjusted_y)[3] == 0 {
                        continue;
                    }
                    accumulate_adjusted_pixel_deletion(
                        source_dimensions,
                        mapper,
                        adjusted_x,
                        adjusted_y,
                        coverage,
                        deletion,
                    );
                }
            }
        }
    }
}

fn accumulate_rectangle_deletion(
    selection: (u32, u32, u32, u32),
    layer: &Layer,
    source_dimensions: (u32, u32),
    adjusted_dimensions: (u32, u32),
    mapper: spectrum_imaging::AdjustedPixelSourceMapper,
    preview: &RgbaImage,
    deletion: &mut [u8],
) {
    let footprint = transformed_footprint(adjusted_dimensions, layer.transform);
    let selection_left = selection.0 as f32 - 0.5;
    let selection_top = selection.1 as f32 - 0.5;
    let selection_right = selection.0.saturating_add(selection.2) as f32 - 0.5;
    let selection_bottom = selection.1.saturating_add(selection.3) as f32 - 0.5;
    if raster_geometry_is_identity(&layer.adjustments)
        && selection_left <= footprint.0
        && selection_top <= footprint.1
        && selection_right >= footprint.2
        && selection_bottom >= footprint.3
    {
        for (index, pixel) in preview.pixels().enumerate() {
            if pixel[3] != 0 {
                deletion[index] = 255;
            }
        }
        return;
    }

    for adjusted_y in 0..adjusted_dimensions.1 {
        for adjusted_x in 0..adjusted_dimensions.0 {
            if preview.get_pixel(adjusted_x, adjusted_y)[3] == 0
                || !source_pixel_intersects_selection(
                    adjusted_x,
                    adjusted_y,
                    adjusted_dimensions,
                    layer.transform,
                    selection,
                )
            {
                continue;
            }
            accumulate_adjusted_pixel_deletion(
                source_dimensions,
                mapper,
                adjusted_x,
                adjusted_y,
                255,
                deletion,
            );
        }
    }
}

fn accumulate_adjusted_pixel_deletion(
    source_dimensions: (u32, u32),
    mapper: spectrum_imaging::AdjustedPixelSourceMapper,
    adjusted_x: u32,
    adjusted_y: u32,
    coverage: u8,
    deletion: &mut [u8],
) {
    mapper.visit_source_samples(adjusted_x, adjusted_y, |source_x, source_y| {
        let index =
            (u64::from(source_y) * u64::from(source_dimensions.0) + u64::from(source_x)) as usize;
        deletion[index] = deletion[index].max(coverage);
    });
}

fn raster_geometry_is_identity(adjustments: &spectrum_imaging::Adjustments) -> bool {
    adjustments.rotation == 0
        && !adjustments.flip_horizontal
        && !adjustments.flip_vertical
        && adjustments.straighten.abs() <= 0.01
        && adjustments.crop.is_none()
}

fn transformed_footprint(dimensions: (u32, u32), transform: Transform) -> (f32, f32, f32, f32) {
    let scaled = (
        scaled_dimension(dimensions.0, transform.scale_x),
        scaled_dimension(dimensions.1, transform.scale_y),
    );
    let bounds =
        crate::render_region::centered_rotation_bounds(scaled.0, scaled.1, transform.rotation);
    let left = (transform.x + bounds.offset_x).round();
    let top = (transform.y + bounds.offset_y).round();
    (
        left - 0.5,
        top - 0.5,
        left + bounds.width as f32 - 0.5,
        top + bounds.height as f32 - 0.5,
    )
}

fn source_pixel_intersects_selection(
    x: u32,
    y: u32,
    dimensions: (u32, u32),
    transform: Transform,
    selection: (u32, u32, u32, u32),
) -> bool {
    let scaled = (
        scaled_dimension(dimensions.0, transform.scale_x),
        scaled_dimension(dimensions.1, transform.scale_y),
    );
    let scale = (
        scaled.0 as f32 / dimensions.0 as f32,
        scaled.1 as f32 / dimensions.1 as f32,
    );
    let bounds =
        crate::render_region::centered_rotation_bounds(scaled.0, scaled.1, transform.rotation);
    let origin = (
        (transform.x + bounds.offset_x).round(),
        (transform.y + bounds.offset_y).round(),
    );
    let center = ((scaled.0 as f32 - 1.0) * 0.5, (scaled.1 as f32 - 1.0) * 0.5);
    let (sin, cos) = crate::transform_math::rotation_sin_cos(transform.rotation);
    let transform_point = |point: (f32, f32)| {
        let dx = point.0 - center.0;
        let dy = point.1 - center.1;
        (
            origin.0 + cos * dx - sin * dy + (bounds.width as f32 - 1.0) * 0.5,
            origin.1 + sin * dx + cos * dy + (bounds.height as f32 - 1.0) * 0.5,
        )
    };
    let left = x as f32 * scale.0 - 0.5;
    let top = y as f32 * scale.1 - 0.5;
    let right = (x + 1) as f32 * scale.0 - 0.5;
    let bottom = (y + 1) as f32 * scale.1 - 0.5;
    let quad = [
        transform_point((left, top)),
        transform_point((right, top)),
        transform_point((right, bottom)),
        transform_point((left, bottom)),
    ];
    convex_quad_intersects_rectangle(quad, selection)
}

fn convex_quad_intersects_rectangle(
    quad: [(f32, f32); 4],
    rectangle: (u32, u32, u32, u32),
) -> bool {
    let left = rectangle.0 as f32 - 0.5;
    let top = rectangle.1 as f32 - 0.5;
    let right = rectangle.0.saturating_add(rectangle.2) as f32 - 0.5;
    let bottom = rectangle.1.saturating_add(rectangle.3) as f32 - 0.5;
    let rect = [(left, top), (right, top), (right, bottom), (left, bottom)];
    let edge = (quad[1].0 - quad[0].0, quad[1].1 - quad[0].1);
    let side = (quad[3].0 - quad[0].0, quad[3].1 - quad[0].1);
    [(1.0, 0.0), (0.0, 1.0), (-edge.1, edge.0), (-side.1, side.0)]
        .into_iter()
        .all(|axis| projections_overlap(&quad, &rect, axis))
}

fn projections_overlap(left: &[(f32, f32); 4], right: &[(f32, f32); 4], axis: (f32, f32)) -> bool {
    let project = |point: &(f32, f32)| point.0 * axis.0 + point.1 * axis.1;
    let bounds = |points: &[(f32, f32); 4]| {
        points.iter().map(project).fold(
            (f32::INFINITY, f32::NEG_INFINITY),
            |(minimum, maximum), value| (minimum.min(value), maximum.max(value)),
        )
    };
    let (left_min, left_max) = bounds(left);
    let (right_min, right_max) = bounds(right);
    left_max > right_min && right_max > left_min
}

pub(crate) fn apply_pixel_mask_region(
    image: &mut RgbaImage,
    mask: Option<&PixelMask>,
    full_dimensions: (u32, u32),
    origin: (u32, u32),
) -> Result<()> {
    let Some(mask) = mask else {
        return Ok(());
    };
    if (mask.width, mask.height) != full_dimensions {
        bail!("pixel mask dimensions do not match the raster source");
    }
    if origin.0.saturating_add(image.width()) > mask.width
        || origin.1.saturating_add(image.height()) > mask.height
    {
        bail!("pixel mask region exceeds the raster source");
    }
    for (local_x, local_y, pixel) in image.enumerate_pixels_mut() {
        let source_x = origin.0 + local_x;
        let source_y = origin.1 + local_y;
        let index = (u64::from(source_y) * u64::from(mask.width) + u64::from(source_x)) as usize;
        pixel[3] = multiply_alpha(pixel[3], mask.alpha[index]);
    }
    Ok(())
}

pub(crate) fn apply_adjusted_pixel_mask_region(
    image: &mut RgbaImage,
    mask: Option<&PixelMask>,
    source_dimensions: (u32, u32),
    adjustments: &spectrum_imaging::Adjustments,
    adjusted_dimensions: (u32, u32),
    origin: (u32, u32),
) -> Result<()> {
    let Some(mask) = mask else {
        return Ok(());
    };
    validate_pixel_mask_source_dimensions(mask, source_dimensions)?;
    if raster_geometry_is_identity(adjustments) {
        return apply_pixel_mask_region(image, Some(mask), source_dimensions, origin);
    }
    let geometry = geometry_adjustments(adjustments);
    let region = spectrum_imaging::PixelRegion {
        x: origin.0,
        y: origin.1,
        width: image.width(),
        height: image.height(),
    };
    if origin.0.saturating_add(image.width()) > adjusted_dimensions.0
        || origin.1.saturating_add(image.height()) > adjusted_dimensions.1
    {
        bail!("pixel mask region exceeds the adjusted raster source");
    }
    let (transformed, _) = spectrum_imaging::render_image_region_at_source_resolution_bounded(
        source_dimensions.0,
        source_dimensions.1,
        geometry,
        region,
        crate::MAX_COLOR_SELECTION_PIXELS,
        |requested| {
            Ok::<_, std::convert::Infallible>(RgbaImage::from_fn(
                requested.width,
                requested.height,
                |x, y| {
                    let source_x = requested.x + x;
                    let source_y = requested.y + y;
                    let index = (u64::from(source_y) * u64::from(mask.width) + u64::from(source_x))
                        as usize;
                    Rgba([255, 255, 255, mask.alpha[index]])
                },
            ))
        },
    )
    .map_err(anyhow::Error::new)?;
    for (pixel, mask_pixel) in image.pixels_mut().zip(transformed.pixels()) {
        pixel[3] = multiply_alpha(pixel[3], mask_pixel[3]);
    }
    Ok(())
}

pub(crate) fn apply_pixel_mask_to_adjusted_preview(
    layer: &Layer,
    image: &mut RgbaImage,
    max_size: Option<u32>,
) -> Result<()> {
    let (Some(mask), LayerKind::Raster { .. }) = (&layer.pixel_mask, &layer.kind) else {
        return Ok(());
    };
    let mask_image = RgbaImage::from_fn(mask.width, mask.height, |x, y| {
        let index = (u64::from(y) * u64::from(mask.width) + u64::from(x)) as usize;
        Rgba([255, 255, 255, mask.alpha[index]])
    });
    let transformed = spectrum_imaging::render_image(
        DynamicImage::ImageRgba8(mask_image),
        geometry_adjustments(&layer.adjustments),
        spectrum_imaging::RenderOptions { max_size },
    )
    .into_rgba8();
    if transformed.dimensions() != image.dimensions() {
        bail!("adjusted pixel mask dimensions do not match the raster preview");
    }
    for (pixel, mask_pixel) in image.pixels_mut().zip(transformed.pixels()) {
        pixel[3] = multiply_alpha(pixel[3], mask_pixel[3]);
    }
    Ok(())
}

fn validate_pixel_mask_source_dimensions(
    mask: &PixelMask,
    source_dimensions: (u32, u32),
) -> Result<()> {
    if (mask.width, mask.height) != source_dimensions {
        bail!("pixel mask dimensions do not match the raster source");
    }
    Ok(())
}

fn geometry_adjustments(
    adjustments: &spectrum_imaging::Adjustments,
) -> spectrum_imaging::Adjustments {
    spectrum_imaging::Adjustments {
        rotation: adjustments.rotation,
        flip_horizontal: adjustments.flip_horizontal,
        flip_vertical: adjustments.flip_vertical,
        straighten: adjustments.straighten,
        crop: adjustments.crop,
        ..Default::default()
    }
}

fn multiply_alpha(left: u8, right: u8) -> u8 {
    ((u16::from(left) * u16::from(right) + 127) / 255) as u8
}

fn raster_source_sample_ranges(
    source: (u32, u32),
    transform: Transform,
    canvas_x: u32,
    canvas_y: u32,
) -> Option<(std::ops::Range<u32>, std::ops::Range<u32>)> {
    let scaled = (
        scaled_dimension(source.0, transform.scale_x),
        scaled_dimension(source.1, transform.scale_y),
    );
    let (output, offset, direction) = if transform.rotation.abs() >= 0.01 {
        let bounds =
            crate::render_region::centered_rotation_bounds(scaled.0, scaled.1, transform.rotation);
        (
            (bounds.width, bounds.height),
            (bounds.offset_x, bounds.offset_y),
            Some(crate::transform_math::rotation_sin_cos(transform.rotation)),
        )
    } else {
        (scaled, (0.0, 0.0), None)
    };
    let origin_x = (transform.x + offset.0).round() as i64;
    let origin_y = (transform.y + offset.1).round() as i64;
    let output_x = i64::from(canvas_x) - origin_x;
    let output_y = i64::from(canvas_y) - origin_y;
    if output_x < 0
        || output_y < 0
        || output_x >= i64::from(output.0)
        || output_y >= i64::from(output.1)
    {
        return None;
    }
    let scaled_coordinate = direction.map_or_else(
        || Some((output_x as u32, output_y as u32)),
        |direction| {
            inverse_center_rotation(output_x as u32, output_y as u32, output, scaled, direction)
        },
    )?;
    Some((
        source_sample_range(source.0, scaled.0, scaled_coordinate.0),
        source_sample_range(source.1, scaled.1, scaled_coordinate.1),
    ))
}

fn scaled_dimension(value: u32, scale: f32) -> u32 {
    (value as f32 * scale).round().max(1.0) as u32
}

fn inverse_center_rotation(
    output_x: u32,
    output_y: u32,
    output: (u32, u32),
    source: (u32, u32),
    direction: (f32, f32),
) -> Option<(u32, u32)> {
    let source_center = ((source.0 as f32 - 1.0) * 0.5, (source.1 as f32 - 1.0) * 0.5);
    let output_center = ((output.0 - 1) as f32 * 0.5, (output.1 - 1) as f32 * 0.5);
    let dx = output_x as f32 - output_center.0;
    let dy = output_y as f32 - output_center.1;
    let source_x = direction.1 * dx + direction.0 * dy + source_center.0;
    let source_y = -direction.0 * dx + direction.1 * dy + source_center.1;
    if source_x < 0.0
        || source_y < 0.0
        || source_x >= source.0 as f32
        || source_y >= source.1 as f32
    {
        return None;
    }
    Some((
        source_x
            .round()
            .clamp(0.0, source.0.saturating_sub(1) as f32) as u32,
        source_y
            .round()
            .clamp(0.0, source.1.saturating_sub(1) as f32) as u32,
    ))
}

fn source_sample_range(source: u32, output: u32, coordinate: u32) -> std::ops::Range<u32> {
    if source == output {
        return coordinate..coordinate + 1;
    }
    let ratio = source as f32 / output as f32;
    let scale = ratio.max(1.0);
    let input = (coordinate as f32 + 0.5) * ratio;
    let start = ((input - scale).floor() as i64).clamp(0, i64::from(source) - 1) as u32;
    let end = ((input + scale).ceil() as i64).clamp(i64::from(start) + 1, i64::from(source)) as u32;
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_coverage_is_half_open_at_adjacent_pixel_edges() {
        assert!(source_pixel_intersects_selection(
            0,
            0,
            (2, 1),
            Transform::default(),
            (0, 0, 1, 1)
        ));
        assert!(!source_pixel_intersects_selection(
            1,
            0,
            (2, 1),
            Transform::default(),
            (0, 0, 1, 1)
        ));
    }

    #[test]
    fn rotated_scaled_adjusted_footprint_does_not_include_touching_rectangle() {
        let transform = Transform {
            x: 10.0,
            y: 12.0,
            scale_x: 2.0,
            scale_y: 2.0,
            rotation: 90.0,
        };
        let adjusted_after_crop = (2, 3);
        let footprint = transformed_footprint(adjusted_after_crop, transform);
        let touching_x = (footprint.2 + 0.5).round() as u32;
        assert!(((touching_x as f32 - 0.5) - footprint.2).abs() < 0.001);
        let top = (footprint.1 + 0.5).round().max(0.0) as u32;
        let height = (footprint.3 - footprint.1).ceil() as u32;
        for y in 0..adjusted_after_crop.1 {
            for x in 0..adjusted_after_crop.0 {
                assert!(!source_pixel_intersects_selection(
                    x,
                    y,
                    adjusted_after_crop,
                    transform,
                    (touching_x, top, 1, height)
                ));
            }
        }
    }
}
