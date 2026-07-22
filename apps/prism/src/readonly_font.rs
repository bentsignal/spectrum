use std::path::Path;

use anyhow::{Context, Result, bail};
use spectrum_revisions::{Asset, RevisionStore};

use crate::{
    Command, Document, FontAsset, Layer, LayerKind, LayerTransfer, LayerTransferFont, ShapeStroke,
    Transform, VerifiedFontSource, apply_command,
};

use super::{APPLICATION_ID, AssetReference, PrismCompatibility, decode_snapshot};

pub struct ReadOnlyFontSource {
    pub font: FontAsset,
    pub source: VerifiedFontSource,
    pub embedded_font_count: usize,
    pub next_font_id: u64,
}

pub struct ReadOnlyFontSubsetInput {
    pub document: Document,
    pub source: VerifiedFontSource,
}

pub fn inspect_font_source_read_only(path: &Path, font_id: u64) -> Result<ReadOnlyFontSource> {
    let (store, document) = load_font_document_read_only(path)?;
    let font = document.font_asset(font_id)?.clone();
    let asset = font_asset(&store, &font.path)?;
    let source = font.verify_embedded_bytes(asset.bytes)?;
    Ok(ReadOnlyFontSource {
        font,
        source,
        embedded_font_count: document.font_assets.len(),
        next_font_id: document.next_font_id,
    })
}

pub fn inspect_font_subset_read_only(path: &Path, font_id: u64) -> Result<ReadOnlyFontSubsetInput> {
    let (store, document) = load_font_document_read_only(path)?;
    let font = document.font_asset(font_id)?;
    let asset = font_asset(&store, &font.path)?;
    let source = font.verify_embedded_bytes(asset.bytes)?;
    Ok(ReadOnlyFontSubsetInput { document, source })
}

fn load_font_document_read_only(path: &Path) -> Result<(RevisionStore, Document)> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let store = RevisionStore::open_read_only(&absolute)?;
    let info = store.project_info()?;
    if info.application_id != APPLICATION_ID {
        bail!(
            "{} is a {} project, not a Prism project",
            path.display(),
            info.application_id
        );
    }
    let latest = store.most_recent_cursor_for_track(info.default_track_id)?;
    let cursor = store.newest_compatible_ancestor(latest, &PrismCompatibility)?;
    let plan = store.replay_plan(cursor, &PrismCompatibility)?;
    let snapshot_bytes = decode_snapshot(&plan.snapshot)?;
    let mut document: Document =
        serde_json::from_slice(&snapshot_bytes).context("invalid Prism snapshot")?;
    hydrate_read_only_legacy_font_permissions(&store, &mut document)?;
    document.migrate()?;
    for step in plan.steps {
        let commands: Vec<Command> = serde_json::from_slice(&step.operations.bytes)
            .context("invalid Prism operation batch")?;
        for command in commands {
            replay_command(&store, &mut document, command)?;
        }
    }
    document.migrate()?;
    Ok((store, document))
}

fn hydrate_read_only_legacy_font_permissions(
    store: &RevisionStore,
    document: &mut Document,
) -> Result<()> {
    for font in &mut document.font_assets {
        if font.embedding_permission != crate::FontEmbeddingPermission::LegacyUnknown {
            continue;
        }
        let asset = font_asset(store, &font.path)?;
        let source = font.verify_embedded_bytes(asset.bytes)?;
        font.hydrate_legacy_from_verified(&source);
    }
    Ok(())
}

fn replay_command(store: &RevisionStore, document: &mut Document, command: Command) -> Result<()> {
    match command {
        Command::ImportFont { path, source_name } => {
            replay_font_import(store, document, &path, source_name.as_deref())
        }
        Command::AddRaster { path, name, x, y } => {
            let id = document.allocate_id();
            document.layers.push(Layer {
                id,
                name: name.unwrap_or_else(|| {
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                }),
                transform: Transform {
                    x,
                    y,
                    ..Default::default()
                },
                kind: LayerKind::Raster {
                    path,
                    original_path: None,
                },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(())
        }
        Command::InsertLayer { transfer, index } => {
            replay_layer_transfer(store, document, *transfer, index)
        }
        Command::RasterizeShape { id, path, scale } => {
            if !scale.is_finite() || scale <= 0.0 {
                bail!("stored rasterization scale is invalid");
            }
            let layer = document.layer_mut(id)?;
            if !matches!(
                layer.kind,
                LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. }
            ) {
                bail!("stored rasterization target is not a shape layer");
            }
            layer.kind = LayerKind::Raster {
                path,
                original_path: None,
            };
            layer.transform.scale_x /= scale;
            layer.transform.scale_y /= scale;
            layer.stroke = ShapeStroke::default();
            layer.shape_fill = None;
            layer.pixel_mask = None;
            Ok(())
        }
        command => {
            apply_command(document, command)?;
            Ok(())
        }
    }
}

fn replay_layer_transfer(
    store: &RevisionStore,
    document: &mut Document,
    transfer: LayerTransfer,
    index: Option<usize>,
) -> Result<()> {
    transfer.validate_envelope()?;
    let LayerTransfer {
        mut layer,
        font_asset,
        ..
    } = transfer;
    let transferred_font_id = font_asset
        .map(|font| replay_transferred_font(store, document, font))
        .transpose()?;
    match &mut layer.kind {
        LayerKind::Text { typography, .. } => typography.font_id = transferred_font_id,
        _ if transferred_font_id.is_some() => {
            bail!("stored non-text layer transfer contains a font asset");
        }
        _ => {}
    }
    let id = document.allocate_id();
    layer.id = id;
    let default_index = document
        .selected
        .and_then(|selected| {
            document
                .layers
                .iter()
                .position(|candidate| candidate.id == selected)
                .map(|position| position + 1)
        })
        .unwrap_or(document.layers.len());
    document.layers.insert(
        index.unwrap_or(default_index).min(document.layers.len()),
        layer,
    );
    document.selected = Some(id);
    Ok(())
}

fn replay_font_import(
    store: &RevisionStore,
    document: &mut Document,
    path: &Path,
    source_name: Option<&str>,
) -> Result<()> {
    let asset = font_asset(store, path)?;
    let reference = AssetReference::parse(path).context("font operation has no asset identity")?;
    let source_name = source_name
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}.{}", reference.id, reference.extension));
    let font = FontAsset::from_embedded_bytes(
        document.next_font_id,
        source_name,
        path.to_owned(),
        asset.bytes,
        &reference.id.to_string(),
    )?;
    if document
        .font_assets
        .iter()
        .all(|existing| existing.content_hash != font.content_hash)
    {
        document.next_font_id += 1;
        document.font_assets.push(font);
    }
    Ok(())
}

fn replay_transferred_font(
    store: &RevisionStore,
    document: &mut Document,
    transferred: LayerTransferFont,
) -> Result<u64> {
    let asset = font_asset(store, &transferred.path)?;
    let reference = AssetReference::parse(&transferred.path)
        .context("transferred font has no asset identity")?;
    let parsed = FontAsset::from_embedded_bytes(
        document.next_font_id,
        transferred.source_name.clone(),
        transferred.path,
        asset.bytes,
        &reference.id.to_string(),
    )?;
    if parsed.family != transferred.family
        || parsed.style != transferred.style
        || parsed.weight != transferred.weight
        || parsed.slant != transferred.slant
        || parsed.subset_allowed != transferred.subset_allowed
        || parsed.content_hash != transferred.content_hash
    {
        bail!("transferred font metadata does not match its OpenType bytes");
    }
    if let Some(existing) = document
        .font_assets
        .iter()
        .find(|existing| existing.content_hash == parsed.content_hash)
    {
        return Ok(existing.id);
    }
    let id = parsed.id;
    document.next_font_id += 1;
    document.font_assets.push(parsed);
    Ok(id)
}

fn font_asset(store: &RevisionStore, path: &Path) -> Result<Asset> {
    let reference = AssetReference::parse(path).context("font is not an embedded project asset")?;
    store
        .asset_record(reference.id)?
        .with_context(|| format!("embedded Prism font asset {} is missing", reference.id))
}
