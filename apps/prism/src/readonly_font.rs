use std::path::Path;

use anyhow::{Context, Result, bail};
use spectrum_revisions::{Asset, RevisionStore};

use crate::{Command, Document, FontAsset, LayerTransferFont, VerifiedFontSource};

use super::{APPLICATION_ID, AssetReference, PrismCompatibility, decode_snapshot};

pub struct ReadOnlyFontSource {
    pub font: FontAsset,
    pub source: VerifiedFontSource,
}

pub fn inspect_font_source_read_only(path: &Path, font_id: u64) -> Result<ReadOnlyFontSource> {
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
    document.migrate()?;
    for step in plan.steps {
        let commands: Vec<Command> = serde_json::from_slice(&step.operations.bytes)
            .context("invalid Prism operation batch")?;
        for command in commands {
            match command {
                Command::ImportFont { path } => {
                    replay_font_import(&store, &mut document, &path)?;
                }
                Command::InsertLayer { transfer, .. } => {
                    if let Some(font) = transfer.font_asset {
                        replay_transferred_font(&store, &mut document, font)?;
                    }
                }
                _ => {}
            }
        }
    }
    let font = document.font_asset(font_id)?.clone();
    let asset = font_asset(&store, &font.path)?;
    let source = font.verify_embedded_bytes(asset.bytes)?;
    Ok(ReadOnlyFontSource { font, source })
}

fn replay_font_import(store: &RevisionStore, document: &mut Document, path: &Path) -> Result<()> {
    let asset = font_asset(store, path)?;
    let reference = AssetReference::parse(path).context("font operation has no asset identity")?;
    let source_name = format!("{}.{}", reference.id, reference.extension);
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
) -> Result<()> {
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
    if document
        .font_assets
        .iter()
        .all(|existing| existing.content_hash != parsed.content_hash)
    {
        document.next_font_id += 1;
        document.font_assets.push(parsed);
    }
    Ok(())
}

fn font_asset(store: &RevisionStore, path: &Path) -> Result<Asset> {
    let reference = AssetReference::parse(path).context("font is not an embedded project asset")?;
    store
        .asset_record(reference.id)?
        .with_context(|| format!("embedded Prism font asset {} is missing", reference.id))
}
