use anyhow::{Context, Result};

use crate::{CommandOutput, Document, Layer, LayerKind, PixelMask, Transform, commands::output};

pub(crate) fn fill_selection(
    document: &mut Document,
    color: [u8; 4],
    name: Option<String>,
) -> Result<CommandOutput> {
    let selection = document
        .selection
        .clone()
        .context("create a rectangular selection before filling")?
        .validated(document.width, document.height)?;
    let (x, y, width, height) = selection.bounds();
    let pixel_mask = selection
        .shared_alpha()
        .map(|alpha| PixelMask::new(width, height, alpha));
    document.validate_projected_inline_mask_budget(
        document.selection.as_ref(),
        pixel_mask.as_ref().map_or(0, |mask| mask.alpha.len()),
    )?;
    let id = document.allocate_id();
    document.layers.push(Layer {
        id,
        name: name.unwrap_or_else(|| "Fill".into()),
        transform: Transform {
            x: x as f32,
            y: y as f32,
            ..Default::default()
        },
        kind: LayerKind::Rectangle {
            width,
            height,
            color,
            corner_radius: 0.0,
        },
        pixel_mask,
        ..Default::default()
    });
    document.selected = Some(id);
    document.validate_inline_mask_budget()?;
    Ok(output(
        "fill_selection",
        "created nondestructive fill layer",
        vec![id],
    ))
}

pub(crate) fn delete_selected_pixels(document: &mut Document, id: u64) -> Result<CommandOutput> {
    crate::pixel_masks::delete_selected_raster_pixels(document, id)?;
    Ok(output(
        "delete_selected_pixels",
        "deleted selected raster pixels nondestructively",
        vec![id],
    ))
}
