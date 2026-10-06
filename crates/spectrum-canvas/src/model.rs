//! A canvas as a durable document: how its commands apply, which files it
//! uses (images, fonts, Clone Stamp sources), and how commands that depend
//! on the canvas's current state are resolved before they are stored.
use anyhow::{Result, bail};
use spectrum_document::{FileRole, FileVisitor, Model, Step};

use crate::{Command, CommandOutput, Document, LayerKind, PaintSelection, apply_command};

/// Spectrum's canvas documents.
pub struct CanvasModel;

/// A canvas, in memory or durably in its revision file.
pub type Workspace = spectrum_document::Workspace<CanvasModel>;

impl Model for CanvasModel {
    type Document = Document;
    type Command = Command;
    type Output = CommandOutput;

    const APPLICATION: &'static str = "spectrum.canvas";
    const NOUN: &'static str = "canvas";

    fn apply(document: &mut Document, command: Command) -> Result<CommandOutput> {
        apply_command(document, command)
    }

    fn describe(output: &CommandOutput) -> String {
        output.message.clone()
    }

    fn stepped(step: Step) -> CommandOutput {
        match step {
            Step::Undo => crate::commands::output("undo", "went back one edit", Vec::new()),
            Step::Redo => crate::commands::output("redo", "went forward one edit", Vec::new()),
        }
    }

    fn step(command: &Command) -> Option<Step> {
        match command {
            Command::Undo => Some(Step::Undo),
            Command::Redo => Some(Step::Redo),
            _ => None,
        }
    }

    fn transient(command: &Command) -> bool {
        matches!(command, Command::SelectLayer { .. })
    }

    fn document_files(document: &mut Document, visit: &mut FileVisitor) -> Result<()> {
        for layer in &mut document.layers {
            if let LayerKind::Raster { path, .. } = &mut layer.kind {
                visit(path, FileRole::Content)?;
            }
        }
        for font in &mut document.font_assets {
            visit(
                &mut font.path,
                FileRole::Identified(font.content_hash.clone()),
            )?;
        }
        for source in document.sampled_sources.values_mut() {
            visit(
                &mut source.path,
                FileRole::Identified(source.content_hash.clone()),
            )?;
        }
        Ok(())
    }

    fn command_files(command: &mut Command, visit: &mut FileVisitor) -> Result<()> {
        match command {
            Command::AddRaster { path, .. }
            | Command::AddLinkedImage { path, .. }
            | Command::RasterizeShape { path, .. }
            | Command::ImportFont { path, .. } => visit(path, FileRole::Content)?,
            Command::SetCloneSource {
                resolved_source: Some(source),
                ..
            } => visit(
                &mut source.path,
                FileRole::Identified(source.content_hash.clone()),
            )?,
            Command::InsertLayer { transfer, .. } => {
                if let LayerKind::Raster { path, .. } = &mut transfer.layer.kind {
                    visit(path, FileRole::Content)?;
                }
                if let Some(font) = &mut transfer.font_asset {
                    visit(&mut font.path, FileRole::Content)?;
                }
                for source in transfer.sampled_sources.values_mut() {
                    visit(
                        &mut source.path,
                        FileRole::Identified(source.content_hash.clone()),
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Captures what a command reads from the canvas (a Clone Stamp source,
    /// a magic wand's selection, the selection a stroke paints within) so
    /// the stored command replays exactly, and names imported fonts.
    fn prepare(document: &Document, commands: &mut [Command]) -> Result<()> {
        let mut candidate = document.clone();
        for command in commands.iter_mut() {
            resolve(&candidate, command)?;
            apply_command(&mut candidate, command.clone())?;
        }
        Ok(())
    }

    /// A magic wand's selection depends on pixels that may come from
    /// outside the canvas, so it is stored as a snapshot rather than replayed.
    fn stored(command: &Command) -> (Command, bool) {
        match command {
            Command::MagicWandSelection {
                x,
                y,
                tolerance,
                contiguous,
                antialias,
                ..
            }
            | Command::MagicWandSnapshot {
                x,
                y,
                tolerance,
                contiguous,
                antialias,
            } => (
                Command::MagicWandSnapshot {
                    x: *x,
                    y: *y,
                    tolerance: *tolerance,
                    contiguous: *contiguous,
                    antialias: *antialias,
                },
                true,
            ),
            command => (command.clone(), false),
        }
    }

    /// The CurrentClone marker is resolved before a stroke is stored, so a
    /// stored stroke carrying it was not written by Spectrum.
    fn check_stored(command: &Command) -> Result<()> {
        let marked = match command {
            Command::AddBrushStroke { stroke, .. }
            | Command::AddPaintLayerWithStroke { stroke, .. } => {
                matches!(stroke.source, Some(crate::SampledBrushSource::CurrentClone))
            }
            Command::InsertLayer { transfer, .. } => matches!(
                &transfer.layer.kind,
                LayerKind::Paint { program } if program.contains_current_clone_marker()
            ),
            _ => false,
        };
        if marked {
            bail!("stored canvas edits cannot contain the authoring-only CurrentClone marker");
        }
        Ok(())
    }

    fn loaded(document: &mut Document) -> Result<()> {
        document.validate()
    }
}

fn resolve(candidate: &Document, command: &mut Command) -> Result<()> {
    match command {
        Command::ImportFont { path, source_name } if source_name.is_none() => {
            *source_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned);
        }
        Command::SetCloneSource {
            id,
            document_x,
            document_y,
            resolved_source,
        } if resolved_source.is_none() => {
            *resolved_source = Some(Box::new(crate::SampledSourceSnapshot::capture(
                candidate.layer(*id)?,
                [*document_x, *document_y],
            )?));
        }
        Command::MagicWandSelection {
            x,
            y,
            tolerance,
            contiguous,
            antialias,
            resolved_selection,
        } if resolved_selection.is_none() => {
            *resolved_selection = Some(Box::new(crate::magic_wand_selection(
                candidate,
                *x,
                *y,
                *tolerance,
                *contiguous,
                *antialias,
            )?));
        }
        Command::AddBrushStroke {
            id,
            stroke,
            selection,
        } => {
            let layer = candidate.layer(*id)?;
            let LayerKind::Paint { program } = &layer.kind else {
                bail!("layer {id} is not a Paint layer");
            };
            *stroke = stroke.resolve_current_clone(
                current_clone(candidate),
                (program.width, program.height),
                layer.transform,
            )?;
            snapshot_selection(candidate, selection);
        }
        Command::AddPaintLayerWithStroke {
            stroke,
            selection,
            width,
            height,
            ..
        } => {
            *stroke = stroke.resolve_current_clone(
                current_clone(candidate),
                (*width, *height),
                crate::Transform::default(),
            )?;
            snapshot_selection(candidate, selection);
        }
        _ => {}
    }
    Ok(())
}

fn current_clone(
    candidate: &Document,
) -> Option<(&crate::SampledSourceId, &crate::SampledSourceSnapshot)> {
    let id = candidate.clone_source.as_ref()?;
    candidate.sampled_sources.get(id).map(|source| (id, source))
}

fn snapshot_selection(candidate: &Document, selection: &mut PaintSelection) {
    if matches!(selection, PaintSelection::Current) {
        *selection = candidate
            .selection
            .clone()
            .map_or(PaintSelection::None, |selection| PaintSelection::Snapshot {
                selection: Box::new(selection),
            });
    }
}
