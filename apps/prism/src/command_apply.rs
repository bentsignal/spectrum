use std::fs;

use super::*;
use crate::commands::{guide_output, output};
use crate::text::default_text_layer_name;

#[derive(Clone, Copy)]
enum SelectionAfterCrop {
    Intersect,
    Clear,
}

fn crop_canvas(
    document: &mut Document,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    selection_after_crop: SelectionAfterCrop,
) -> Result<()> {
    if width == 0 || height == 0 || x >= document.width || y >= document.height {
        bail!("crop must overlap the canvas and have a nonzero size");
    }
    let width = width.min(document.width - x);
    let height = height.min(document.height - y);
    let selection = match selection_after_crop {
        SelectionAfterCrop::Intersect => document
            .selection
            .clone()
            .and_then(|selection| selection.cropped(x, y, width, height)),
        SelectionAfterCrop::Clear => None,
    };
    document.width = width;
    document.height = height;
    document.selection = selection;
    for layer in &mut document.layers {
        layer.transform.x -= x as f32;
        layer.transform.y -= y as f32;
    }
    alignment::crop_guides(document, x, y);
    Ok(())
}

pub(super) fn apply_command(document: &mut Document, command: Command) -> Result<CommandOutput> {
    match command {
        Command::RenameDocument { name } => {
            let name = name.trim();
            if name.is_empty() {
                bail!("document name cannot be empty");
            }
            if name.chars().count() > MAX_DOCUMENT_NAME_CHARS {
                bail!("document name cannot exceed {MAX_DOCUMENT_NAME_CHARS} characters");
            }
            if name.chars().any(char::is_control) {
                bail!("document name cannot contain control characters");
            }
            document.name = name.into();
            Ok(output("rename_document", "renamed document", Vec::new()))
        }
        Command::SetCanvas {
            width,
            height,
            background,
        } => {
            document.width = width.clamp(1, MAX_CANVAS_DIMENSION);
            document.height = height.clamp(1, MAX_CANVAS_DIMENSION);
            document.background = background;
            document.selection = document
                .selection
                .take()
                .and_then(|selection| selection.clipped(document.width, document.height));
            alignment::clamp_guides(document);
            Ok(output("set_canvas", "updated canvas", Vec::new()))
        }
        Command::CropCanvas {
            x,
            y,
            width,
            height,
        } => {
            crop_canvas(document, x, y, width, height, SelectionAfterCrop::Intersect)?;
            Ok(output("crop_canvas", "cropped canvas", Vec::new()))
        }
        Command::CropToSelection => {
            let selection = document
                .selection
                .clone()
                .context("create a rectangular selection before cropping")?
                .validated(document.width, document.height)?;
            let (x, y, width, height) = selection.bounds();
            if (x, y, width, height) == (0, 0, document.width, document.height) {
                bail!("selection already covers the full canvas");
            }
            crop_canvas(document, x, y, width, height, SelectionAfterCrop::Clear)?;
            Ok(output(
                "crop_to_selection",
                "cropped canvas to selection",
                Vec::new(),
            ))
        }
        Command::AddRaster { path, name, x, y } => {
            require_finite("x", x)?;
            require_finite("y", y)?;
            let path = fs::canonicalize(&path)
                .with_context(|| format!("could not open raster layer {}", path.display()))?;
            image::ImageReader::open(&path)
                .with_context(|| format!("could not open {}", path.display()))?
                .with_guessed_format()?
                .into_dimensions()
                .with_context(|| format!("could not inspect {}", path.display()))?;
            let id = document.allocate_id();
            let layer_name = name.unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
            document.layers.push(Layer {
                id,
                name: layer_name,
                transform: Transform {
                    x,
                    y,
                    ..Default::default()
                },
                kind: LayerKind::Raster {
                    original_path: Some(path.clone()),
                    path,
                },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(output("add_raster", "added raster layer", vec![id]))
        }
        Command::AddText {
            text,
            name,
            font_size,
            color,
            x,
            y,
        } => {
            require_finite("font size", font_size)?;
            require_finite("x", x)?;
            require_finite("y", y)?;
            if text.trim().is_empty() {
                bail!("text cannot be empty");
            }
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
                    typography: TextTypography::default(),
                },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(output("add_text", "added text layer", vec![id]))
        }
        Command::ImportFont { path, source_name } => {
            let id = document.next_font_id;
            let mut font = FontAsset::import(id, &path)?;
            if let Some(source_name) = source_name {
                font.source_name = source_name;
            }
            if let Some(existing) = document
                .font_assets
                .iter()
                .find(|existing| existing.content_hash == font.content_hash)
            {
                return Ok(output(
                    "import_font",
                    &format!("font already embedded as asset {}", existing.id),
                    Vec::new(),
                ));
            }
            document.next_font_id += 1;
            let message = format!("embedded {} {} as font asset {id}", font.family, font.style);
            document.font_assets.push(font);
            Ok(output("import_font", &message, Vec::new()))
        }
        Command::AddRectangle {
            name,
            width,
            height,
            color,
            corner_radius,
            x,
            y,
        } => {
            require_finite("corner radius", corner_radius)?;
            require_finite("x", x)?;
            require_finite("y", y)?;
            let id = document.allocate_id();
            document.layers.push(Layer {
                id,
                name: name.unwrap_or_else(|| "Rectangle".into()),
                transform: Transform {
                    x,
                    y,
                    ..Default::default()
                },
                kind: LayerKind::Rectangle {
                    width: width.clamp(1, MAX_CANVAS_DIMENSION),
                    height: height.clamp(1, MAX_CANVAS_DIMENSION),
                    color,
                    corner_radius: corner_radius.max(0.0),
                },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(output("add_rectangle", "added rectangle layer", vec![id]))
        }
        Command::AddEllipse {
            name,
            width,
            height,
            color,
            x,
            y,
        } => {
            require_finite("x", x)?;
            require_finite("y", y)?;
            let id = document.allocate_id();
            document.layers.push(Layer {
                id,
                name: name.unwrap_or_else(|| "Ellipse".into()),
                transform: Transform {
                    x,
                    y,
                    ..Default::default()
                },
                kind: LayerKind::Ellipse {
                    width: width.clamp(1, MAX_CANVAS_DIMENSION),
                    height: height.clamp(1, MAX_CANVAS_DIMENSION),
                    color,
                },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(output("add_ellipse", "added ellipse layer", vec![id]))
        }
        Command::AddPath {
            name,
            geometry,
            color,
            x,
            y,
        } => {
            require_finite("x", x)?;
            require_finite("y", y)?;
            let id = document.allocate_id();
            document.layers.push(Layer {
                id,
                name: name.unwrap_or_else(|| "Path".into()),
                transform: Transform {
                    x,
                    y,
                    ..Default::default()
                },
                stroke: ShapeStroke {
                    enabled: true,
                    width: 2.0,
                    color,
                },
                kind: LayerKind::Path { geometry, color },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(output("add_path", "added path layer", vec![id]))
        }
        Command::AddPaintLayer {
            name,
            width,
            height,
        } => {
            let program = BrushProgram::new(width, height)?;
            document.validate_projected_paint_budget(None, &program)?;
            let id = document.allocate_id();
            document.layers.push(Layer {
                id,
                name: name.unwrap_or_else(|| "Paint".into()),
                kind: LayerKind::Paint { program },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(output("add_paint_layer", "added Paint layer", vec![id]))
        }
        Command::AddPaintLayerWithStroke {
            name,
            width,
            height,
            stroke,
            selection,
        } => {
            let program = BrushProgram::new(width, height)?;
            let validated_snapshot = match &selection {
                PaintSelection::Snapshot { selection } => Some(
                    selection
                        .as_ref()
                        .clone()
                        .validated(document.width, document.height)?,
                ),
                _ => None,
            };
            let selection = match &selection {
                PaintSelection::Current => document.selection.as_ref(),
                PaintSelection::None => None,
                PaintSelection::Snapshot { .. } => validated_snapshot.as_ref(),
            };
            let stroke = stroke.validated_for_viewport(width, height)?;
            let clip = crate::paint_selection::capture_selection_clip(
                selection,
                &stroke,
                (width, height),
                Transform::default(),
            )?;
            let stroke = stroke.with_clip(clip, (width, height))?;
            document.validate_projected_inline_mask_budget(
                document.selection.as_ref(),
                stroke.clip.as_ref().map_or(0, BrushClip::byte_len),
            )?;
            let program = program.append(stroke)?;
            document.validate_projected_paint_budget(None, &program)?;
            let id = document.allocate_id();
            document.layers.push(Layer {
                id,
                name: name.unwrap_or_else(|| "Paint".into()),
                kind: LayerKind::Paint { program },
                ..Default::default()
            });
            document.selected = Some(id);
            document.validate_inline_mask_budget()?;
            Ok(output(
                "add_paint_layer_with_stroke",
                "created Paint layer and painted one stroke",
                vec![id],
            ))
        }
        Command::AddBrushStroke {
            id,
            stroke,
            selection,
        } => {
            let (program, transform, locked, direct_coordinates) = {
                let layer = document.layer(id)?;
                let LayerKind::Paint { program } = &layer.kind else {
                    bail!("layer {id} is not a Paint layer");
                };
                (
                    program.clone(),
                    layer.transform,
                    layer.locked,
                    crate::paint_layer_allows_direct_strokes(layer),
                )
            };
            if locked {
                bail!("layer {id} is locked");
            }
            if !direct_coordinates {
                bail!(
                    "Paint layer {id} has geometric adjustments; reset them before Brush/Eraser strokes and reapply them afterward"
                );
            }
            let validated_snapshot = match &selection {
                PaintSelection::Snapshot { selection } => Some(
                    selection
                        .as_ref()
                        .clone()
                        .validated(document.width, document.height)?,
                ),
                _ => None,
            };
            let selection = match &selection {
                PaintSelection::Current => document.selection.as_ref(),
                PaintSelection::None => None,
                PaintSelection::Snapshot { .. } => validated_snapshot.as_ref(),
            };
            let stroke = stroke.validated_for_viewport(program.width, program.height)?;
            let clip = crate::paint_selection::capture_selection_clip(
                selection,
                &stroke,
                (program.width, program.height),
                transform,
            )?;
            let stroke = stroke.with_clip(clip, (program.width, program.height))?;
            let additional_clip_bytes = stroke.clip.as_ref().map_or(0, BrushClip::byte_len);
            document.validate_projected_inline_mask_budget(
                document.selection.as_ref(),
                additional_clip_bytes,
            )?;
            let appended = program.append(stroke)?;
            document.validate_projected_paint_budget(Some(&program), &appended)?;
            document.layer_mut(id)?.kind = LayerKind::Paint { program: appended };
            document.validate_inline_mask_budget()?;
            Ok(output("add_brush_stroke", "painted one stroke", vec![id]))
        }
        Command::UpdateText {
            id,
            text,
            font_size,
            color,
        } => {
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
        Command::SetTextTypography { id, typography } => {
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
            let layer = document.layer_mut(id)?;
            let LayerKind::Text {
                typography: layer_typography,
                ..
            } = &mut layer.kind
            else {
                bail!("layer {id} is not a text layer");
            };
            *layer_typography = typography.sanitized();
            Ok(output(
                "set_text_typography",
                "updated typography",
                vec![id],
            ))
        }
        Command::UpdateRectangle {
            id,
            width,
            height,
            color,
            corner_radius,
        } => {
            require_finite("corner radius", corner_radius)?;
            let width = width.clamp(1, MAX_CANVAS_DIMENSION);
            let height = height.clamp(1, MAX_CANVAS_DIMENSION);
            if let Some(mask) = &document.layer(id)?.pixel_mask
                && (mask.width, mask.height) != (width, height)
            {
                bail!(
                    "resize pixel-masked layer {id} with its transform instead of changing shape dimensions"
                );
            }
            let layer = document.layer_mut(id)?;
            let LayerKind::Rectangle {
                width: layer_width,
                height: layer_height,
                color: layer_color,
                corner_radius: layer_radius,
            } = &mut layer.kind
            else {
                bail!("layer {id} is not a rectangle layer");
            };
            *layer_width = width;
            *layer_height = height;
            *layer_color = color;
            *layer_radius = corner_radius.max(0.0);
            Ok(output(
                "update_rectangle",
                "updated rectangle layer",
                vec![id],
            ))
        }
        Command::UpdateEllipse {
            id,
            width,
            height,
            color,
        } => {
            let width = width.clamp(1, MAX_CANVAS_DIMENSION);
            let height = height.clamp(1, MAX_CANVAS_DIMENSION);
            if let Some(mask) = &document.layer(id)?.pixel_mask
                && (mask.width, mask.height) != (width, height)
            {
                bail!(
                    "resize pixel-masked layer {id} with its transform instead of changing shape dimensions"
                );
            }
            let layer = document.layer_mut(id)?;
            let LayerKind::Ellipse {
                width: layer_width,
                height: layer_height,
                color: layer_color,
            } = &mut layer.kind
            else {
                bail!("layer {id} is not an ellipse layer");
            };
            *layer_width = width;
            *layer_height = height;
            *layer_color = color;
            Ok(output("update_ellipse", "updated ellipse layer", vec![id]))
        }
        Command::ReplacePath { id, geometry } => {
            if document.layer(id)?.pixel_mask.is_some() {
                bail!("path layers cannot carry pixel masks");
            }
            let layer = document.layer_mut(id)?;
            if layer.locked {
                bail!("layer {id} is locked");
            }
            let LayerKind::Path {
                geometry: layer_geometry,
                ..
            } = &mut layer.kind
            else {
                bail!("layer {id} is not a path layer");
            };
            if !geometry.closed() {
                layer.shape_fill = None;
            }
            *layer_geometry = geometry;
            Ok(output("replace_path", "updated path geometry", vec![id]))
        }
        Command::RemoveLayer { id } => {
            let index = document
                .layers
                .iter()
                .position(|layer| layer.id == id)
                .with_context(|| format!("layer {id} is not in this document"))?;
            document.layers.remove(index);
            if document.selected == Some(id) {
                document.selected = document.layers.last().map(|layer| layer.id);
            }
            Ok(output("remove_layer", "removed layer", vec![id]))
        }
        Command::DuplicateLayer { id } => {
            let mut layer = document.layer(id)?.clone();
            let paint_clip_bytes = match &layer.kind {
                LayerKind::Paint { program } => {
                    document.validate_projected_paint_budget(None, program)?;
                    program.clip_bytes()
                }
                _ => 0,
            };
            document.validate_projected_inline_mask_budget(
                document.selection.as_ref(),
                layer
                    .pixel_mask
                    .as_ref()
                    .map_or(0, |mask| mask.alpha.len())
                    .checked_add(paint_clip_bytes)
                    .context("duplicated layer inline mask bytes overflowed")?,
            )?;
            let new_id = document.allocate_id();
            layer.id = new_id;
            layer.name = format!("{} copy", layer.name);
            layer.transform.x += 16.0;
            layer.transform.y += 16.0;
            let index = document
                .layers
                .iter()
                .position(|layer| layer.id == id)
                .unwrap_or(document.layers.len());
            document.layers.insert(index + 1, layer);
            document.selected = Some(new_id);
            document.validate_inline_mask_budget()?;
            Ok(output("duplicate_layer", "duplicated layer", vec![new_id]))
        }
        Command::InsertLayer { transfer, index } => {
            let paint_clip_bytes = match &transfer.layer.kind {
                LayerKind::Paint { program } => {
                    document.validate_projected_paint_budget(None, program)?;
                    program.clip_bytes()
                }
                _ => 0,
            };
            document.validate_projected_inline_mask_budget(
                document.selection.as_ref(),
                transfer
                    .layer
                    .pixel_mask
                    .as_ref()
                    .map_or(0, |mask| mask.alpha.len())
                    .checked_add(paint_clip_bytes)
                    .context("transferred layer inline mask bytes overflowed")?,
            )?;
            let id = (*transfer).insert_into(document, index)?;
            document.validate_inline_mask_budget()?;
            Ok(output(
                "insert_layer",
                "inserted transferred layer",
                vec![id],
            ))
        }
        Command::RenameLayer { id, name } => {
            let name = name.trim();
            if name.is_empty() {
                bail!("layer name cannot be empty");
            }
            document.layer_mut(id)?.name = name.into();
            Ok(output("rename_layer", "renamed layer", vec![id]))
        }
        Command::SelectLayer { id } => {
            if let Some(id) = id {
                document.layer(id)?;
            }
            document.selected = id;
            Ok(output(
                "select_layer",
                "selected layer",
                id.into_iter().collect(),
            ))
        }
        Command::SetSelection { selection } => {
            let selection = selection
                .map(|selection| selection.validated(document.width, document.height))
                .transpose()?;
            document.validate_projected_inline_mask_budget(selection.as_ref(), 0)?;
            document.selection = selection;
            Ok(output(
                "set_selection",
                if document.selection.is_some() {
                    "updated pixel selection"
                } else {
                    "cleared selection"
                },
                Vec::new(),
            ))
        }
        Command::MagicWandSelection {
            x,
            y,
            tolerance,
            contiguous,
            antialias,
            resolved_selection,
        } => {
            let selection = match resolved_selection {
                Some(selection) => selection.validated(document.width, document.height)?,
                None => {
                    crate::magic_wand_selection(document, x, y, tolerance, contiguous, antialias)?
                }
            };
            document.validate_projected_inline_mask_budget(Some(&selection), 0)?;
            document.selection = Some(selection);
            Ok(output(
                "magic_wand_selection",
                "selected pixels by color",
                Vec::new(),
            ))
        }
        Command::MagicWandSnapshot { .. } => {
            bail!("magic wand snapshot markers require their same-revision document snapshot")
        }
        Command::LassoSelection {
            points,
            mode,
            antialias,
        } => {
            let selection = crate::apply_lasso_selection(
                document.selection.as_ref(),
                document.width,
                document.height,
                &points,
                mode,
                antialias,
            )?;
            document.validate_projected_inline_mask_budget(selection.as_ref(), 0)?;
            document.selection = selection;
            Ok(output(
                "lasso_selection",
                if document.selection.is_some() {
                    "updated pixel selection with lasso"
                } else {
                    "lasso cleared the pixel selection"
                },
                Vec::new(),
            ))
        }
        Command::FillSelection { color, name } => {
            crate::selection_commands::fill_selection(document, color, name)
        }
        Command::DeleteSelectedPixels { id } => {
            crate::selection_commands::delete_selected_pixels(document, id)
        }
        Command::MoveLayer { id, index } => {
            let current = document
                .layers
                .iter()
                .position(|layer| layer.id == id)
                .with_context(|| format!("layer {id} is not in this document"))?;
            let layer = document.layers.remove(current);
            let index = index.min(document.layers.len());
            document.layers.insert(index, layer);
            Ok(output("move_layer", "reordered layer", vec![id]))
        }
        Command::SetVisibility { id, visible } => {
            document.layer_mut(id)?.visible = visible;
            Ok(output("set_visibility", "updated visibility", vec![id]))
        }
        Command::SetLocked { id, locked } => {
            document.layer_mut(id)?.locked = locked;
            Ok(output("set_locked", "updated layer lock", vec![id]))
        }
        Command::SetOpacity { id, opacity } => {
            require_finite("opacity", opacity)?;
            document.layer_mut(id)?.opacity = opacity.clamp(0.0, 1.0);
            Ok(output("set_opacity", "updated opacity", vec![id]))
        }
        Command::SetBlendMode { id, blend_mode } => {
            document.layer_mut(id)?.blend_mode = blend_mode;
            Ok(output("set_blend_mode", "updated blend mode", vec![id]))
        }
        Command::SetTransform { id, transform } => {
            validate_transform(transform)?;
            let layer = document.layer_mut(id)?;
            if layer.locked {
                bail!("layer {id} is locked");
            }
            layer.transform = transform.sanitized();
            Ok(output("set_transform", "transformed layer", vec![id]))
        }
        Command::SetRotation { id, degrees } => {
            require_finite("rotation", degrees)?;
            let layer = document.layer_mut(id)?;
            if layer.locked {
                bail!("layer {id} is locked");
            }
            layer.transform.rotation = degrees.rem_euclid(360.0);
            Ok(output("set_rotation", "rotated layer", vec![id]))
        }
        Command::AlignLayer {
            id,
            alignment,
            reference,
        } => {
            let transform = align_layer_transform(document, id, alignment, reference)?;
            document.layer_mut(id)?.transform = transform.sanitized();
            Ok(output("align_layer", "aligned layer", vec![id]))
        }
        Command::SetSnapping { enabled } => {
            document.snapping_enabled = enabled;
            Ok(output(
                "set_snapping",
                if enabled {
                    "enabled snapping"
                } else {
                    "disabled snapping"
                },
                Vec::new(),
            ))
        }
        Command::AddGuide {
            orientation,
            position,
        } => {
            let id = alignment::add_guide(document, orientation, position)?;
            Ok(guide_output("add_guide", "added guide", vec![id]))
        }
        Command::MoveGuide { id, position } => {
            alignment::move_guide(document, id, position)?;
            Ok(guide_output("move_guide", "moved guide", vec![id]))
        }
        Command::RemoveGuide { id } => {
            alignment::remove_guide(document, id)?;
            Ok(guide_output("remove_guide", "removed guide", vec![id]))
        }
        Command::AdjustLayer { id, patch } => {
            let layer = document.layer_mut(id)?;
            let mut adjustments = layer.adjustments.clone();
            patch.apply_to(&mut adjustments);
            validate_adjustments(&adjustments)?;
            layer.adjustments = adjustments;
            Ok(output("adjust_layer", "adjusted layer", vec![id]))
        }
        Command::ResetLayerAdjustments { id } => {
            let layer = document.layer_mut(id)?;
            layer.adjustments = Adjustments::default();
            Ok(output(
                "reset_layer_adjustments",
                "reset layer adjustments",
                vec![id],
            ))
        }
        Command::SetMask { id, mask } => {
            validate_mask(mask)?;
            document.layer_mut(id)?.mask = mask.sanitized();
            Ok(output("set_mask", "updated layer mask", vec![id]))
        }
        Command::SetVectorMask { id, mask } => {
            if let Some(mask) = &mask {
                mask.validate()?;
            }
            let layer = document.layer_mut(id)?;
            if layer.locked {
                bail!("layer {id} is locked");
            }
            layer.vector_mask = mask;
            Ok(output("set_vector_mask", "updated vector mask", vec![id]))
        }
        Command::SetShapeStroke { id, stroke } => {
            validate_shape_stroke(stroke)?;
            let layer = document.layer_mut(id)?;
            if !matches!(
                layer.kind,
                LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } | LayerKind::Path { .. }
            ) {
                bail!("layer {id} is not a shape layer");
            }
            layer.stroke = stroke.sanitized();
            Ok(output("set_shape_stroke", "updated shape stroke", vec![id]))
        }
        Command::SetLayerStyle { id, style } => {
            effects::validate_layer_style(&style)?;
            let layer = document.layer_mut(id)?;
            if layer.locked {
                bail!("layer {id} is locked");
            }
            layer.style = style.sanitized();
            Ok(output("set_layer_style", "updated layer style", vec![id]))
        }
        Command::SetShapeFill { id, fill } => {
            if let Some(fill) = &fill {
                effects::validate_shape_fill(fill)?;
            }
            let layer = document.layer_mut(id)?;
            if layer.locked {
                bail!("layer {id} is locked");
            }
            if !matches!(
                layer.kind,
                LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } | LayerKind::Path { .. }
            ) {
                bail!("layer {id} is not a shape layer");
            }
            if matches!(&layer.kind, LayerKind::Path { geometry, .. } if !geometry.closed())
                && fill.is_some()
            {
                bail!("open path layers cannot have a shape fill");
            }
            layer.shape_fill = fill.map(ShapeFill::sanitized);
            Ok(output("set_shape_fill", "updated shape fill", vec![id]))
        }
        Command::RasterizeShape { id, path, scale } => {
            require_finite("rasterization scale", scale)?;
            if scale <= 0.0 {
                bail!("rasterization scale must be positive");
            }
            let source_layer = document.layer(id)?;
            let (shape_width, shape_height) =
                shape_dimensions(source_layer).context("layer is not a parametric shape")?;
            let path_origin = crate::paths::path_source_bounds(source_layer)
                .map(|bounds| bounds.origin)
                .unwrap_or([0.0; 2]);
            let path = fs::canonicalize(&path)
                .with_context(|| format!("could not open rasterized shape {}", path.display()))?;
            let (width, height) = image::image_dimensions(&path).with_context(|| {
                format!("could not inspect rasterized shape {}", path.display())
            })?;
            let expected_width = (shape_width as f32 * scale).round().max(1.0) as u32;
            let expected_height = (shape_height as f32 * scale).round().max(1.0) as u32;
            if (width, height) != (expected_width, expected_height) {
                bail!(
                    "rasterized shape is {width}x{height}, expected {expected_width}x{expected_height} at {scale}x"
                );
            }
            let layer = document.layer_mut(id)?;
            layer.kind = LayerKind::Raster {
                path,
                original_path: None,
            };
            layer.transform.x += path_origin[0] * layer.transform.scale_x;
            layer.transform.y += path_origin[1] * layer.transform.scale_y;
            layer.transform.scale_x /= scale;
            layer.transform.scale_y /= scale;
            layer.stroke = ShapeStroke::default();
            layer.shape_fill = None;
            layer.pixel_mask = None;
            layer.vector_mask = None;
            Ok(output(
                "rasterize_shape",
                "rasterized shape layer",
                vec![id],
            ))
        }
        Command::SetClipping { id, enabled } => {
            document.layer_mut(id)?.clip_to_below = enabled;
            Ok(output("set_clipping", "updated clipping", vec![id]))
        }
        Command::Undo | Command::Redo => unreachable!("history is handled by Workspace"),
    }
}
