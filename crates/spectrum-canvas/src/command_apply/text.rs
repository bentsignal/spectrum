use anyhow::{Result, bail};

use crate::{
    CommandOutput, Document, Layer, LayerKind, TextShaping, TextTypography, Transform,
    commands::output, require_finite, text::default_text_layer_name,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn add(
    document: &mut Document,
    text: String,
    name: Option<String>,
    font_size: f32,
    color: [u8; 4],
    x: f32,
    y: f32,
    shaping: TextShaping,
) -> Result<CommandOutput> {
    require_finite("font size", font_size)?;
    require_finite("x", x)?;
    require_finite("y", y)?;
    if text.trim().is_empty() {
        bail!("text cannot be empty");
    }
    let shaping = shaping.validated()?;
    let id = document.allocate_id();
    document.layers.push(Layer {
        id,
        name: name.unwrap_or_else(|| default_text_layer_name(&text)),
        transform: Transform {
            x,
            y,
            ..Default::default()
        },
        kind: LayerKind::Text {
            text,
            font_size: font_size.clamp(4.0, 1_000.0),
            color,
            typography: TextTypography {
                shaping,
                ..TextTypography::default()
            },
        },
        ..Default::default()
    });
    document.selected = Some(id);
    Ok(output("add_text", "added text layer", vec![id]))
}

pub(super) fn update(
    document: &mut Document,
    id: u64,
    text: String,
    font_size: f32,
    color: [u8; 4],
) -> Result<CommandOutput> {
    require_finite("font size", font_size)?;
    if text.trim().is_empty() {
        bail!("text cannot be empty");
    }
    let layer = document.layer_mut(id)?;
    let auto_named = if let LayerKind::Text { text, .. } = &layer.kind {
        layer.name == default_text_layer_name(text)
    } else {
        bail!("layer {id} is not a text layer");
    };
    {
        let LayerKind::Text {
            text: layer_text,
            font_size: layer_size,
            color: layer_color,
            ..
        } = &mut layer.kind
        else {
            unreachable!("text kind was checked above");
        };
        *layer_text = text;
        *layer_size = font_size.clamp(4.0, 1_000.0);
        *layer_color = color;
    }
    if auto_named && let LayerKind::Text { text, .. } = &layer.kind {
        layer.name = default_text_layer_name(text);
    }
    Ok(output("update_text", "updated text layer", vec![id]))
}

pub(super) fn set_typography(
    document: &mut Document,
    id: u64,
    typography: TextTypography,
) -> Result<CommandOutput> {
    require_finite("line height", typography.line_height)?;
    require_finite("tracking", typography.tracking)?;
    require_finite("outline width", typography.effects.outline_width)?;
    require_finite("shadow x", typography.effects.shadow_offset_x)?;
    require_finite("shadow y", typography.effects.shadow_offset_y)?;
    if let Some(width) = typography.box_width {
        require_finite("text box width", width)?;
    }
    if let Some(font_id) = typography.font_id {
        document.font_asset(font_id)?;
    }
    let typography = typography.validated_and_sanitized()?;
    let layer = document.layer_mut(id)?;
    let LayerKind::Text {
        typography: layer_typography,
        ..
    } = &mut layer.kind
    else {
        bail!("layer {id} is not a text layer");
    };
    *layer_typography = typography;
    Ok(output(
        "set_text_typography",
        "updated typography",
        vec![id],
    ))
}
