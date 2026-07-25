use anyhow::Result;

use crate::{Command, Document, SampledSourceSnapshot};

pub(super) fn command_has_sampled_sources(command: &Command) -> bool {
    match command {
        Command::SetCloneSource {
            resolved_source: Some(_),
            ..
        } => true,
        Command::InsertLayer { transfer, .. } => !transfer.sampled_sources.is_empty(),
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
        Command::InsertLayer { transfer, .. } => {
            for source in transfer.sampled_sources.values_mut() {
                map(source)?;
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
    for source in document.sampled_sources.values_mut() {
        map(source)?;
    }
    Ok(())
}
