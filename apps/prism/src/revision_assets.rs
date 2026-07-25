use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use spectrum_revisions::{Asset, AssetId};

use crate::{FontAsset, FontSourceSnapshot, LayerTransferFont};

const ASSET_PREFIX: &str = "spectrum-asset:";

pub(super) struct PreparedAsset {
    pub(super) reference: AssetReference,
    pub(super) asset: Asset,
    pub(super) source_path: PathBuf,
}

pub(crate) const MAX_EMBEDDED_RASTER_BYTES: usize = 512 * 1024 * 1024;

pub(super) fn prepare_asset(path: &Path) -> Result<PreparedAsset> {
    if let Some(reference) = AssetReference::parse(path) {
        bail!(
            "cannot prepare unresolved embedded asset reference {}",
            reference.id
        );
    }
    let (canonical, bytes) = crate::font_source::read_secure_regular_file(
        path,
        MAX_EMBEDDED_RASTER_BYTES,
        "raster asset",
    )
    .with_context(|| format!("could not embed {}", path.display()))?;
    Ok(prepare_asset_bytes(&canonical, bytes))
}

pub(super) fn prepare_generated_asset(path: &Path) -> Result<PreparedAsset> {
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("could not resolve generated asset {}", path.display()))?;
    prepare_asset(&canonical)
}

fn prepare_asset_bytes(path: &Path, bytes: Vec<u8>) -> PreparedAsset {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(sanitize_extension)
        .filter(|extension| !extension.is_empty())
        .unwrap_or_else(|| "bin".into());
    let asset = Asset::new(media_type(&extension), bytes);
    PreparedAsset {
        reference: AssetReference {
            id: asset.id,
            extension,
        },
        asset,
        source_path: path.to_owned(),
    }
}

pub(super) fn prepare_font_snapshot(snapshot: &FontSourceSnapshot) -> Result<PreparedAsset> {
    let prepared = prepare_asset_bytes(snapshot.canonical_path(), snapshot.bytes().to_vec());
    if prepared.asset.id.to_string() != snapshot.content_hash() {
        bail!("font snapshot identity does not match the revision asset identity");
    }
    Ok(prepared)
}

pub(super) fn prepare_verified_font_asset(font: &FontAsset) -> Result<PreparedAsset> {
    prepare_font_snapshot(&font.source_snapshot()?)
}

pub(super) fn prepare_verified_transfer_font_asset(
    font: &LayerTransferFont,
) -> Result<PreparedAsset> {
    let snapshot = FontSourceSnapshot::read_verified(&font.path, &font.content_hash)?;
    if snapshot.family != font.family
        || snapshot.style != font.style
        || snapshot.weight != font.weight
        || snapshot.slant != font.slant
        || snapshot.subset_allowed != font.subset_allowed
    {
        bail!("transferred font metadata does not match its immutable source snapshot");
    }
    prepare_font_snapshot(&snapshot)
}

pub(super) struct AssetReference {
    pub(super) id: AssetId,
    pub(super) extension: String,
}

impl AssetReference {
    pub(super) fn parse(path: &Path) -> Option<Self> {
        let value = path.to_str()?.strip_prefix(ASSET_PREFIX)?;
        let (hash, extension) = value.split_once('.')?;
        let id = AssetId::from_hex(hash)?;
        let extension = sanitize_extension(extension);
        if extension.is_empty() {
            return None;
        }
        Some(Self { id, extension })
    }

    pub(super) fn path(&self) -> PathBuf {
        PathBuf::from(format!("{ASSET_PREFIX}{}.{}", self.id, self.extension))
    }
}

fn sanitize_extension(extension: &str) -> String {
    extension
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>()
        .to_ascii_lowercase()
}

pub(super) fn media_type(extension: &str) -> &'static str {
    match extension {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "tif" | "tiff" => "image/tiff",
        "webp" => "image/webp",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}
