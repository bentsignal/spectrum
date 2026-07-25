use crate::Command;

pub(super) fn requires_modern_encoding(command: &Command) -> bool {
    match command {
        Command::SetShapeFill {
            fill: Some(fill), ..
        } => fill.requires_modern_encoding(),
        Command::InsertLayer { transfer, .. } => {
            transfer.version >= crate::MODERN_GRADIENT_LAYER_TRANSFER_VERSION
                || transfer
                    .layer
                    .shape_fill
                    .as_ref()
                    .is_some_and(crate::ShapeFill::requires_modern_encoding)
        }
        _ => false,
    }
}

pub(super) fn command_uses_clone_stamp(command: &Command) -> bool {
    match command {
        Command::SetCloneSource { .. } => true,
        Command::AddPaintLayerWithStroke { stroke, .. }
        | Command::AddBrushStroke { stroke, .. } => {
            stroke.style.mode == crate::BrushMode::CloneStamp || stroke.source.is_some()
        }
        Command::InsertLayer { transfer, .. } => {
            transfer.version >= crate::CLONE_STAMP_LAYER_TRANSFER_VERSION
                || !transfer.sampled_sources.is_empty()
                || matches!(&transfer.layer.kind, crate::LayerKind::Paint { program } if program.contains_sampled_sources())
        }
        _ => false,
    }
}

pub(crate) fn downgrade_compatible_transfers(commands: &mut [Command]) {
    for command in commands {
        if let Command::InsertLayer { transfer, .. } = command
            && transfer.validate_envelope().is_ok()
        {
            let minimal_version = if transfer
                .layer
                .shape_fill
                .as_ref()
                .is_some_and(crate::ShapeFill::requires_modern_encoding)
            {
                crate::MODERN_GRADIENT_LAYER_TRANSFER_VERSION
            } else if matches!(
                &transfer.layer.kind,
                crate::LayerKind::Paint { program } if program.contains_sampled_sources()
            ) {
                crate::CLONE_STAMP_LAYER_TRANSFER_VERSION
            } else if matches!(
                &transfer.layer.kind,
                crate::LayerKind::Text { typography, .. }
                    if typography.shaping.engine == crate::TextShapingEngine::HarfBuzzV1
            ) {
                crate::SHAPED_TEXT_LAYER_TRANSFER_VERSION
            } else if matches!(transfer.layer.kind, crate::LayerKind::Raster { .. })
                && transfer.layer.pixel_mask.is_some()
            {
                crate::RASTER_PIXEL_MASK_LAYER_TRANSFER_VERSION
            } else if transfer.layer.blend_mode == crate::BlendMode::Dissolve
                || transfer.layer.dissolve_seed != 0
            {
                crate::DISSOLVE_LAYER_TRANSFER_VERSION
            } else if matches!(transfer.layer.kind, crate::LayerKind::Paint { .. }) {
                crate::PAINT_LAYER_TRANSFER_VERSION
            } else if transfer.layer.vector_mask.is_some()
                || matches!(transfer.layer.kind, crate::LayerKind::Path { .. })
            {
                crate::PATH_LAYER_TRANSFER_VERSION
            } else if transfer.layer.pixel_mask.is_some() {
                3
            } else if transfer.layer.style.is_empty() && transfer.layer.shape_fill.is_none() {
                1
            } else {
                2
            };
            transfer.version = transfer.version.min(minimal_version);
        }
    }
}

pub(super) fn command_contains_current_clone_marker(command: &Command) -> bool {
    match command {
        Command::AddPaintLayerWithStroke { stroke, .. }
        | Command::AddBrushStroke { stroke, .. } => {
            matches!(stroke.source, Some(crate::SampledBrushSource::CurrentClone))
        }
        Command::InsertLayer { transfer, .. } => matches!(
            &transfer.layer.kind,
            crate::LayerKind::Paint { program } if program.contains_current_clone_marker()
        ),
        _ => false,
    }
}
