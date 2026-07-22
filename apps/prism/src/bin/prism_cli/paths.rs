use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use prism_core::{Command, PathGeometry, VectorMask};

const MAX_PATH_JSON_BYTES: u64 = 512 * 1024;

#[derive(Args)]
pub(super) struct PathArgs {
    #[command(subcommand)]
    pub command: PathCommand,
}

#[derive(Subcommand)]
pub(super) enum PathCommand {
    /// Add an editable cubic path from bounded PathGeometry JSON.
    Add {
        geometry: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "ffffffff")]
        color: String,
        #[arg(long, default_value_t = 0.0)]
        x: f32,
        #[arg(long, default_value_t = 0.0)]
        y: f32,
    },
    /// Replace one path layer's anchor and control-point geometry.
    Replace { id: u64, geometry: PathBuf },
}

#[derive(Args)]
pub(super) struct VectorMaskArgs {
    pub id: u64,
    /// Closed, nondegenerate PathGeometry JSON to stretch over the target layer.
    pub geometry: Option<PathBuf>,
    #[arg(long)]
    pub invert: bool,
    #[arg(long, conflicts_with = "geometry")]
    pub clear: bool,
}

pub(super) fn replace_command(id: u64, geometry: PathBuf) -> Result<Command> {
    Ok(Command::ReplacePath {
        id,
        geometry: read_geometry(geometry)?,
    })
}

pub(super) fn vector_mask_command(arguments: VectorMaskArgs) -> Result<Command> {
    if arguments.clear {
        return Ok(Command::SetVectorMask {
            id: arguments.id,
            mask: None,
        });
    }
    let geometry = arguments
        .geometry
        .ok_or_else(|| anyhow::anyhow!("vector-mask requires geometry JSON or --clear"))?;
    Ok(Command::SetVectorMask {
        id: arguments.id,
        mask: Some(VectorMask::new(read_geometry(geometry)?, arguments.invert)?),
    })
}

pub(super) fn read_geometry(path: PathBuf) -> Result<PathGeometry> {
    let metadata =
        fs::metadata(&path).with_context(|| format!("inspect path geometry {}", path.display()))?;
    if !metadata.is_file() {
        bail!("path geometry must be a regular file");
    }
    if metadata.len() > MAX_PATH_JSON_BYTES {
        bail!("path geometry JSON exceeds {MAX_PATH_JSON_BYTES} bytes");
    }
    let bytes =
        fs::read(&path).with_context(|| format!("read path geometry {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("decode path geometry {}", path.display()))
}
