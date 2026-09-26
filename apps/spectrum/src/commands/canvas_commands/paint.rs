use std::{fs::File, io::Read, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use prism_core::{BrushStroke, Command, PaintSelection};

pub(super) const MAX_BRUSH_STROKE_JSON_BYTES: usize = 32 * 1024 * 1024;

#[derive(Args)]
pub(super) struct PaintArgs {
    #[command(subcommand)]
    pub command: PaintCommand,
}

#[derive(Subcommand)]
pub(super) enum PaintCommand {
    /// Add an empty nondestructive Paint layer.
    AddLayer {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
    },
    /// Append one bounded BrushStroke JSON value to an existing Paint layer.
    Stroke {
        id: u64,
        stroke: PathBuf,
        /// Ignore the current document selection for this stroke.
        #[arg(long)]
        no_selection: bool,
    },
    /// Capture an immutable Clone Stamp source from a Raster layer.
    CloneSource {
        id: u64,
        document_x: f32,
        document_y: f32,
    },
    /// Append one Clone Stamp stroke using the current immutable source.
    CloneStroke {
        id: u64,
        stroke: PathBuf,
        /// Ignore the current document selection for this stroke.
        #[arg(long)]
        no_selection: bool,
    },
}

pub(super) fn paint_command(arguments: PaintArgs) -> Result<Command> {
    Ok(match arguments.command {
        PaintCommand::AddLayer {
            name,
            width,
            height,
        } => Command::AddPaintLayer {
            name,
            width,
            height,
        },
        PaintCommand::Stroke {
            id,
            stroke,
            no_selection,
        } => Command::AddBrushStroke {
            id,
            stroke: read_stroke(&stroke)?,
            selection: if no_selection {
                PaintSelection::None
            } else {
                PaintSelection::Current
            },
        },
        PaintCommand::CloneSource {
            id,
            document_x,
            document_y,
        } => Command::SetCloneSource {
            id,
            document_x,
            document_y,
            resolved_source: None,
        },
        PaintCommand::CloneStroke {
            id,
            stroke,
            no_selection,
        } => Command::AddBrushStroke {
            id,
            stroke: read_stroke(&stroke)?.as_current_clone()?,
            selection: if no_selection {
                PaintSelection::None
            } else {
                PaintSelection::Current
            },
        },
    })
}

fn read_stroke(path: &PathBuf) -> Result<BrushStroke> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("could not open brush stroke {}", path.display()))?
        .take((MAX_BRUSH_STROKE_JSON_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("could not read brush stroke {}", path.display()))?;
    if bytes.len() > MAX_BRUSH_STROKE_JSON_BYTES {
        anyhow::bail!("BrushStroke JSON exceeds the 32 MiB input limit");
    }
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid BrushStroke JSON {}", path.display()))
}
