use super::*;
use crate::font_source::MAX_EMBEDDED_FONT_BYTES;

pub(crate) struct PreparedEdit {
    pub(super) execution_commands: Vec<Command>,
    provenance: Vec<CommandProvenance>,
    pub(super) operations: PreparedOperations,
    pub(super) force_snapshot: bool,
}

enum CommandProvenance {
    None,
    Raster {
        original_path: PathBuf,
    },
    Font {
        staged_path: PathBuf,
        original_path: PathBuf,
        source_name: String,
        content_hash: String,
    },
}

impl PreparedEdit {
    pub(super) fn new(project: &DurableProject, commands: &[Command]) -> Result<Self> {
        if commands.is_empty() {
            bail!("cannot prepare an empty Prism action");
        }
        preflight_asset_batch(commands)?;
        let mut portable_commands = commands.to_vec();
        let mut execution_commands = commands.to_vec();
        let mut provenance = Vec::with_capacity(commands.len());
        let mut assets = Vec::new();
        let mut force_snapshot = false;

        for (index, command) in commands.iter().enumerate() {
            if let Command::MagicWandSelection {
                x,
                y,
                tolerance,
                contiguous,
                antialias,
                resolved_selection: Some(_),
            } = command
            {
                portable_commands[index] = Command::MagicWandSnapshot {
                    x: *x,
                    y: *y,
                    tolerance: *tolerance,
                    contiguous: *contiguous,
                    antialias: *antialias,
                };
                force_snapshot = true;
            }
            let command_provenance = match command {
                Command::AddRaster { path, name, .. } => {
                    let prepared = prepare_asset(path)?;
                    let original_path = prepared.source_path.clone();
                    let staged_path =
                        project.stage_asset(&prepared.reference, &prepared.asset.bytes)?;
                    let normalized_name = Some(name.clone().unwrap_or_else(|| {
                        original_path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    }));
                    set_add_raster_paths(
                        &mut portable_commands[index],
                        prepared.reference.path(),
                        normalized_name.clone(),
                    );
                    set_add_raster_paths(
                        &mut execution_commands[index],
                        staged_path,
                        normalized_name,
                    );
                    assets.push(prepared.asset);
                    CommandProvenance::Raster { original_path }
                }
                Command::ImportFont { path, .. } => {
                    let snapshot = FontSourceSnapshot::read(path)?;
                    let original_path = snapshot.canonical_path().to_owned();
                    let source_name = original_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("font")
                        .to_owned();
                    let prepared = prepare_font_snapshot(&snapshot)?;
                    let staged_path =
                        project.stage_asset(&prepared.reference, &prepared.asset.bytes)?;
                    set_import_font_path(
                        &mut portable_commands[index],
                        prepared.reference.path(),
                        &source_name,
                    );
                    set_import_font_path(
                        &mut execution_commands[index],
                        staged_path.clone(),
                        &source_name,
                    );
                    assets.push(prepared.asset);
                    CommandProvenance::Font {
                        staged_path,
                        original_path,
                        source_name,
                        content_hash: snapshot.content_hash().to_owned(),
                    }
                }
                Command::RasterizeShape { path, .. } => {
                    let prepared = prepare_generated_asset(path)?;
                    let staged_path =
                        project.stage_asset(&prepared.reference, &prepared.asset.bytes)?;
                    set_rasterized_shape_path(
                        &mut portable_commands[index],
                        prepared.reference.path(),
                    );
                    set_rasterized_shape_path(&mut execution_commands[index], staged_path);
                    assets.push(prepared.asset);
                    CommandProvenance::None
                }
                Command::InsertLayer { transfer, .. } => {
                    let raster_asset = if let LayerKind::Raster { path, .. } = &transfer.layer.kind
                    {
                        Some(prepare_staged_asset(project, path)?)
                    } else {
                        None
                    };
                    let font_asset = transfer
                        .font_asset
                        .as_ref()
                        .map(|font| prepare_staged_font_asset(project, font))
                        .transpose()?;
                    prepare_layer_transfer_command(
                        &mut portable_commands[index],
                        &mut execution_commands[index],
                        raster_asset,
                        font_asset,
                        &mut assets,
                    );
                    CommandProvenance::None
                }
                _ => CommandProvenance::None,
            };
            provenance.push(command_provenance);
        }

        downgrade_compatible_transfers(&mut portable_commands);
        let version = operations_version(&portable_commands);
        Ok(Self {
            execution_commands,
            provenance,
            operations: PreparedOperations::from_portable(version, portable_commands, assets)?,
            force_snapshot,
        })
    }

    pub(crate) fn apply(&self, document: &mut Document) -> Result<Vec<crate::CommandOutput>> {
        let mut outputs = Vec::with_capacity(self.execution_commands.len());
        for (command, provenance) in self.execution_commands.iter().zip(&self.provenance) {
            let font_count_before = document.font_assets.len();
            let output = apply_command(document, command.clone())?;
            match provenance {
                CommandProvenance::None => {}
                CommandProvenance::Raster { original_path } => {
                    for id in &output.layer_ids {
                        if let LayerKind::Raster {
                            original_path: live_original,
                            ..
                        } = &mut document.layer_mut(*id)?.kind
                        {
                            *live_original = Some(original_path.clone());
                        }
                    }
                }
                CommandProvenance::Font {
                    staged_path,
                    original_path,
                    source_name,
                    content_hash,
                } => {
                    if document.font_assets.len() > font_count_before {
                        let font = document
                            .font_assets
                            .iter_mut()
                            .find(|font| font.path == *staged_path)
                            .context("prepared font import did not create its staged asset")?;
                        if font.content_hash != *content_hash {
                            bail!("staged font bytes changed after their verified snapshot");
                        }
                        font.source_name.clone_from(source_name);
                        font.original_path = Some(original_path.clone());
                    } else if !document
                        .font_assets
                        .iter()
                        .any(|font| font.content_hash == *content_hash)
                    {
                        bail!("prepared font import did not retain its verified snapshot");
                    }
                }
            }
            outputs.push(output);
        }
        Ok(outputs)
    }
}

const MAX_PREPARED_ASSET_BATCH_COUNT: usize = 128;
const MAX_PREPARED_ASSET_BATCH_BYTES: u64 = MAX_EMBEDDED_RASTER_BYTES as u64;

fn preflight_asset_batch(commands: &[Command]) -> Result<()> {
    let mut sources = Vec::new();
    for command in commands {
        match command {
            Command::AddRaster { path, .. } => {
                sources.push((path.clone(), MAX_EMBEDDED_RASTER_BYTES, "raster asset"));
            }
            Command::ImportFont { path, .. } => {
                sources.push((path.clone(), MAX_EMBEDDED_FONT_BYTES, "font"));
            }
            Command::RasterizeShape { path, .. } => {
                let path = fs::canonicalize(path).with_context(|| {
                    format!("could not resolve generated asset {}", path.display())
                })?;
                sources.push((path, MAX_EMBEDDED_RASTER_BYTES, "raster asset"));
            }
            Command::InsertLayer { transfer, .. } => {
                if let LayerKind::Raster { path, .. } = &transfer.layer.kind {
                    sources.push((path.clone(), MAX_EMBEDDED_RASTER_BYTES, "raster asset"));
                }
                if let Some(font) = &transfer.font_asset {
                    sources.push((font.path.clone(), MAX_EMBEDDED_FONT_BYTES, "font"));
                }
            }
            _ => {}
        }
    }
    if sources.len() > MAX_PREPARED_ASSET_BATCH_COUNT {
        bail!(
            "one Prism action can embed at most {MAX_PREPARED_ASSET_BATCH_COUNT} assets; place fewer files at a time"
        );
    }
    let mut total = 0_u64;
    for (path, per_asset_limit, kind) in sources {
        let len = crate::font_source::secure_regular_file_len(&path, per_asset_limit, kind)
            .map_err(|error| {
                anyhow::anyhow!("could not preflight {}: {error:#}", path.display())
            })?;
        total = total
            .checked_add(len)
            .context("embedded-asset batch size overflowed")?;
        if total > MAX_PREPARED_ASSET_BATCH_BYTES {
            bail!(
                "one Prism action can embed at most {} MiB across all assets; place fewer or smaller files at a time",
                MAX_PREPARED_ASSET_BATCH_BYTES / (1024 * 1024)
            );
        }
    }
    Ok(())
}

fn prepare_staged_asset(project: &DurableProject, path: &Path) -> Result<(PreparedAsset, PathBuf)> {
    let prepared = prepare_asset(path)?;
    let staged_path = project.stage_asset(&prepared.reference, &prepared.asset.bytes)?;
    Ok((prepared, staged_path))
}

fn prepare_staged_font_asset(
    project: &DurableProject,
    font: &LayerTransferFont,
) -> Result<(PreparedAsset, PathBuf)> {
    let prepared = prepare_verified_transfer_font_asset(font)?;
    let staged_path = project.stage_asset(&prepared.reference, &prepared.asset.bytes)?;
    Ok((prepared, staged_path))
}

fn set_add_raster_paths(command: &mut Command, path: PathBuf, name: Option<String>) {
    let Command::AddRaster {
        path: target_path,
        name: target_name,
        ..
    } = command
    else {
        unreachable!("prepared command changed variant")
    };
    *target_path = path;
    *target_name = name;
}

fn set_import_font_path(command: &mut Command, path: PathBuf, original_name: &str) {
    let Command::ImportFont {
        path: target,
        source_name,
    } = command
    else {
        unreachable!("prepared command changed variant")
    };
    *target = path;
    *source_name = Some(original_name.to_owned());
}

fn set_rasterized_shape_path(command: &mut Command, path: PathBuf) {
    let Command::RasterizeShape { path: target, .. } = command else {
        unreachable!("prepared command changed variant")
    };
    *target = path;
}

fn prepare_layer_transfer_command(
    portable_command: &mut Command,
    execution_command: &mut Command,
    raster_asset: Option<(PreparedAsset, PathBuf)>,
    font_asset: Option<(PreparedAsset, PathBuf)>,
    assets: &mut Vec<Asset>,
) {
    let Command::InsertLayer {
        transfer: portable_transfer,
        ..
    } = portable_command
    else {
        unreachable!("portable command changed variant")
    };
    let Command::InsertLayer {
        transfer: execution_transfer,
        ..
    } = execution_command
    else {
        unreachable!("execution command changed variant")
    };
    if let Some((prepared, staged_path)) = raster_asset {
        if let LayerKind::Raster {
            path,
            original_path,
        } = &mut portable_transfer.layer.kind
        {
            *path = prepared.reference.path();
            *original_path = None;
        }
        if let LayerKind::Raster {
            path,
            original_path,
        } = &mut execution_transfer.layer.kind
        {
            *path = staged_path;
            *original_path = None;
        }
        assets.push(prepared.asset);
    }
    if let Some((prepared, staged_path)) = font_asset {
        if let Some(font) = &mut portable_transfer.font_asset {
            font.path = prepared.reference.path();
        }
        if let Some(font) = &mut execution_transfer.font_asset {
            font.path = staged_path;
        }
        assets.push(prepared.asset);
    }
}
