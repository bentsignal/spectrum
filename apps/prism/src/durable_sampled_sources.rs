use anyhow::Result;

use crate::{Command, Document, LayerKind, SampledSourceSnapshot};

pub(super) fn command_has_sampled_sources(command: &Command) -> bool {
    match command {
        Command::SetCloneSource {
            resolved_source: Some(_),
            ..
        } => true,
        Command::AddBrushStroke { stroke, .. }
        | Command::AddPaintLayerWithStroke { stroke, .. } => stroke.sampled_source().is_some(),
        Command::InsertLayer { transfer, .. } => {
            matches!(&transfer.layer.kind, LayerKind::Paint { program } if program.contains_sampled_sources())
        }
        _ => false,
    }
}

pub(super) fn map_command_sampled_sources(
    command: &mut Command,
    mut map: impl FnMut(&mut SampledSourceSnapshot) -> Result<()>,
) -> Result<()> {
    match command {
        Command::SetCloneSource {
            resolved_source: Some(source),
            ..
        } => map(source),
        Command::AddBrushStroke { stroke, .. }
        | Command::AddPaintLayerWithStroke { stroke, .. } => {
            if let Some(source) = stroke.sampled_source_mut() {
                map(source)?;
            }
            Ok(())
        }
        Command::InsertLayer { transfer, .. } => {
            if let LayerKind::Paint { program } = &mut transfer.layer.kind {
                *program = program.map_sampled_sources(map)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(crate) fn map_document_sampled_sources(
    document: &mut Document,
    mut map: impl FnMut(&mut SampledSourceSnapshot) -> Result<()>,
) -> Result<()> {
    if let Some(source) = &mut document.clone_source {
        map(source)?;
    }
    for layer in &mut document.layers {
        if let LayerKind::Paint { program } = &mut layer.kind {
            *program = program.map_sampled_sources(&mut map)?;
        }
    }
    Ok(())
}
