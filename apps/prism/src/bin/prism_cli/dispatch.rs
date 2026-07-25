use anyhow::{Result, bail};
use prism_core::{
    AlignmentReference, BlendMode, Command, Document, LayerMask, ShapeStroke, Transform,
};
use spectrum_imaging::AdjustmentPatch;

use super::{
    CliCommand, GuideCommand, PathCommand, effects, live_bridge::decode_commands, paint,
    parse_color, paths, selection, text_shaping, transfer, updated_typography,
};

pub(super) struct SemanticPlan {
    pub(super) commands: Vec<Command>,
    pub(super) atomic_batch: bool,
}

pub(super) fn semantic_commands(command: CliCommand, document: &Document) -> Result<SemanticPlan> {
    let mut atomic_batch = false;
    let commands = match command {
        CliCommand::RenameDocument { name } => {
            vec![Command::RenameDocument { name }]
        }
        CliCommand::FontImport { path } => {
            vec![Command::ImportFont {
                path,
                source_name: None,
            }]
        }
        CliCommand::Typography(arguments) => {
            let typography = updated_typography(document, &arguments)?;
            vec![Command::SetTextTypography {
                id: arguments.id,
                typography,
            }]
        }
        CliCommand::LayerPaste(arguments) => {
            vec![transfer::paste_command(arguments)?]
        }
        CliCommand::AddImage { path, name, x, y } => {
            vec![Command::AddRaster { path, name, x, y }]
        }
        CliCommand::AddText {
            text,
            name,
            size,
            color,
            x,
            y,
            layout,
            language,
        } => vec![Command::AddText {
            text,
            name,
            font_size: size,
            color: parse_color(&color)?,
            x,
            y,
            shaping: text_shaping(layout, language.as_deref())?,
        }],
        CliCommand::AddRectangle {
            name,
            width,
            height,
            color,
            radius,
            x,
            y,
        } => vec![Command::AddRectangle {
            name,
            width,
            height,
            color: parse_color(&color)?,
            corner_radius: radius,
            x,
            y,
        }],
        CliCommand::AddEllipse {
            name,
            width,
            height,
            color,
            x,
            y,
        } => vec![Command::AddEllipse {
            name,
            width,
            height,
            color: parse_color(&color)?,
            x,
            y,
        }],
        CliCommand::Path(arguments) => {
            vec![match arguments.command {
                PathCommand::Add {
                    geometry,
                    name,
                    color,
                    x,
                    y,
                } => Command::AddPath {
                    name,
                    geometry: paths::read_geometry(geometry)?,
                    color: parse_color(&color)?,
                    x,
                    y,
                },
                PathCommand::Replace { id, geometry } => paths::replace_command(id, geometry)?,
            }]
        }
        CliCommand::Paint(arguments) => {
            vec![paint::paint_command(arguments)?]
        }
        CliCommand::VectorMask(arguments) => {
            vec![paths::vector_mask_command(arguments)?]
        }
        CliCommand::EditText {
            id,
            text,
            size,
            color,
        } => vec![Command::UpdateText {
            id,
            text,
            font_size: size,
            color: parse_color(&color)?,
        }],
        CliCommand::EditRectangle {
            id,
            width,
            height,
            color,
            radius,
        } => vec![Command::UpdateRectangle {
            id,
            width,
            height,
            color: parse_color(&color)?,
            corner_radius: radius,
        }],
        CliCommand::EditEllipse {
            id,
            width,
            height,
            color,
        } => vec![Command::UpdateEllipse {
            id,
            width,
            height,
            color: parse_color(&color)?,
        }],
        CliCommand::Stroke {
            id,
            enabled,
            width,
            color,
        } => vec![Command::SetShapeStroke {
            id,
            stroke: ShapeStroke {
                enabled,
                width,
                color: parse_color(&color)?,
            },
        }],
        CliCommand::Shadow(arguments) => {
            vec![effects::shadow_command(arguments)?]
        }
        CliCommand::Gradient(arguments) => {
            vec![effects::gradient_command(arguments)?]
        }
        CliCommand::RasterizeShape { id, scale } => {
            let layer = document.layer(id)?;
            let scale = scale
                .map(Ok)
                .unwrap_or_else(|| prism_core::recommended_rasterization_scale(layer))?;
            let asset = prism_core::rasterize_shape_asset(document, id, scale)?;
            vec![Command::RasterizeShape {
                id,
                path: asset.path,
                scale: asset.scale,
            }]
        }
        CliCommand::Rename { id, name } => {
            vec![Command::RenameLayer { id, name }]
        }
        CliCommand::Delete { id } => {
            vec![Command::RemoveLayer { id }]
        }
        CliCommand::Duplicate { id } => {
            vec![Command::DuplicateLayer { id }]
        }
        CliCommand::Select { id } => {
            vec![Command::SelectLayer { id }]
        }
        CliCommand::Selection(arguments) => {
            vec![selection::command(arguments)?]
        }
        CliCommand::Reorder { id, index } => {
            vec![Command::MoveLayer { id, index }]
        }
        CliCommand::Visibility { id, visible } => {
            vec![Command::SetVisibility { id, visible }]
        }
        CliCommand::Lock { id, locked } => {
            vec![Command::SetLocked { id, locked }]
        }
        CliCommand::Opacity { id, opacity } => {
            vec![Command::SetOpacity { id, opacity }]
        }
        CliCommand::Blend { id, mode, seed } => {
            atomic_batch = true;
            let blend_mode = BlendMode::from(mode);
            if seed.is_some() && blend_mode != BlendMode::Dissolve {
                bail!("--seed is only valid with the dissolve blend mode");
            }
            let mut commands = vec![Command::SetBlendMode { id, blend_mode }];
            if let Some(seed) = seed {
                commands.push(Command::SetDissolveSeed { id, seed });
            }
            commands
        }
        CliCommand::Transform {
            id,
            x,
            y,
            scale_x,
            scale_y,
            rotation,
        } => vec![Command::SetTransform {
            id,
            transform: Transform {
                x,
                y,
                scale_x,
                scale_y,
                rotation,
            },
        }],
        CliCommand::Rotate { id, degrees } => {
            vec![Command::SetRotation { id, degrees }]
        }
        CliCommand::Align {
            id,
            alignment,
            to_layer,
        } => vec![Command::AlignLayer {
            id,
            alignment: alignment.into(),
            reference: to_layer.map_or(AlignmentReference::Canvas, |id| {
                AlignmentReference::Layer { id }
            }),
        }],
        CliCommand::Snapping { enabled } => {
            vec![Command::SetSnapping { enabled }]
        }
        CliCommand::Guide { command } => vec![match command {
            GuideCommand::Add {
                orientation,
                position,
            } => Command::AddGuide {
                orientation: orientation.into(),
                position,
            },
            GuideCommand::Move { id, position } => Command::MoveGuide { id, position },
            GuideCommand::Remove { id } => Command::RemoveGuide { id },
        }],
        CliCommand::Adjust {
            id,
            exposure,
            contrast,
            highlights,
            shadows,
            temperature,
            tint,
            vibrance,
            saturation,
            clarity,
            dehaze,
            noise_reduction,
            sharpening,
        } => vec![Command::AdjustLayer {
            id,
            patch: AdjustmentPatch {
                exposure,
                contrast,
                highlights,
                shadows,
                temperature,
                tint,
                vibrance,
                saturation,
                clarity,
                dehaze,
                noise_reduction,
                sharpening,
                ..Default::default()
            },
        }],
        CliCommand::ResetAdjustments { id } => {
            vec![Command::ResetLayerAdjustments { id }]
        }
        CliCommand::Mask {
            id,
            x,
            y,
            width,
            height,
            invert,
            clear,
        } => vec![Command::SetMask {
            id,
            mask: LayerMask {
                enabled: !clear,
                x,
                y,
                width,
                height,
                invert,
            },
        }],
        CliCommand::Clip { id, enabled } => {
            vec![Command::SetClipping { id, enabled }]
        }
        CliCommand::Canvas {
            width,
            height,
            background,
        } => vec![Command::SetCanvas {
            width,
            height,
            background: parse_color(&background)?,
        }],
        CliCommand::Crop {
            x,
            y,
            width,
            height,
        } => vec![Command::CropCanvas {
            x,
            y,
            width,
            height,
        }],
        CliCommand::Run { json } => {
            atomic_batch = json.trim_start().starts_with('[');
            decode_commands(&json)?
        }
        CliCommand::Init { .. }
        | CliCommand::List
        | CliCommand::FontList { .. }
        | CliCommand::FontUsage { .. }
        | CliCommand::FontSource { .. }
        | CliCommand::FontSubsetPlan { .. }
        | CliCommand::OptimizedCopy { .. }
        | CliCommand::LayerCopy(..)
        | CliCommand::Export { .. }
        | CliCommand::FromLumen { .. }
        | CliCommand::Agent { .. }
        | CliCommand::Live { .. }
        | CliCommand::Schema
        | CliCommand::Benchmark { .. } => unreachable!(),
    };
    Ok(SemanticPlan {
        commands,
        atomic_batch,
    })
}
