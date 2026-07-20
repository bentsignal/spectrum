use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Args;
use prism_core::{Command, Document, LAYER_TRANSFER_FORMAT, LayerTransfer};
use serde_json::{Value, json};

#[derive(Clone, Debug, Args)]
pub(super) struct LayerCopyArgs {
    /// Layer to copy. Defaults to the selected layer.
    pub id: Option<u64>,
    /// New JSON transfer file to create.
    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Clone, Debug, Args)]
pub(super) struct LayerPasteArgs {
    /// JSON transfer file created by layer-copy.
    pub input: PathBuf,
    /// Bottom-to-top insertion index. Defaults to immediately above selection.
    #[arg(long)]
    pub index: Option<usize>,
}

pub(super) fn copy_layer(document: &Document, arguments: LayerCopyArgs) -> Result<Value> {
    let transfer = match arguments.id {
        Some(id) => LayerTransfer::from_document(document, id)?,
        None => LayerTransfer::from_selected(document)?,
    };
    let json = transfer.to_json_pretty()?;
    write_new(&arguments.output, json.as_bytes())?;
    Ok(json!({
        "ok": true,
        "action": "layer_copy",
        "format": LAYER_TRANSFER_FORMAT,
        "version": transfer.version,
        "layer": transfer.layer.name,
        "output": arguments.output,
    }))
}

pub(super) fn paste_command(arguments: LayerPasteArgs) -> Result<Command> {
    let metadata = fs::metadata(&arguments.input)
        .with_context(|| format!("could not inspect {}", arguments.input.display()))?;
    if metadata.len() > 4 * 1024 * 1024 {
        bail!("Prism layer transfer exceeds the 4 MiB metadata limit");
    }
    let json = fs::read_to_string(&arguments.input)
        .with_context(|| format!("could not read {}", arguments.input.display()))?;
    Ok(Command::InsertLayer {
        transfer: LayerTransfer::from_json(&json)?,
        index: arguments.index,
    })
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("could not create new transfer file {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
