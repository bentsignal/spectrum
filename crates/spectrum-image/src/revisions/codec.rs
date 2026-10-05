use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use spectrum_revisions::{Asset, AssetId, Compatibility, Encoding, Payload};

use crate::{Photo, PhotoBatch, Preset, Project};

pub(super) const CATALOG_TRACK_KIND: &str = "spectrum.lumen.catalog";
pub(super) const PHOTO_TRACK_KIND: &str = "spectrum.lumen.photo";
const CATALOG_SNAPSHOT_FAMILY: &str = "spectrum.lumen.catalog.snapshot";
const CATALOG_OPERATIONS_FAMILY: &str = "spectrum.lumen.catalog.operations";
const PHOTO_SNAPSHOT_FAMILY: &str = "spectrum.lumen.photo.snapshot";
const PHOTO_OPERATIONS_FAMILY: &str = "spectrum.lumen.photo.operations";
const LEGACY_SNAPSHOT_FAMILY: &str = "spectrum.lumen.project";
const LEGACY_OPERATIONS_FAMILY: &str = "spectrum.lumen.commands";
const DEFLATE_CAPABILITY: &str = "deflate";
const VERSION: u32 = 1;
const ASSET_PREFIX: &str = "spectrum-asset:";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct CatalogState {
    version: u32,
    name: String,
    next_id: u64,
    photo_ids: Vec<u64>,
    presets: Vec<Preset>,
    next_preset_id: u64,
    batches: Vec<PhotoBatch>,
    next_batch_id: u64,
}

impl CatalogState {
    pub(super) fn from_project(project: &Project) -> Self {
        Self {
            version: project.version,
            name: project.name.clone(),
            next_id: project.next_id,
            photo_ids: project.photos.iter().map(|photo| photo.id).collect(),
            presets: project.presets.clone(),
            next_preset_id: project.next_preset_id,
            batches: project.batches.clone(),
            next_batch_id: project.next_batch_id,
        }
    }

    pub(super) fn assemble(&self, photos: Vec<Photo>, selected: Option<u64>) -> Project {
        let selected = selected
            .filter(|id| self.photo_ids.contains(id))
            .or_else(|| self.photo_ids.first().copied());
        Project {
            version: self.version,
            name: self.name.clone(),
            next_id: self.next_id,
            selected,
            photos,
            presets: self.presets.clone(),
            next_preset_id: self.next_preset_id,
            batches: self.batches.clone(),
            next_batch_id: self.next_batch_id,
        }
    }

    pub(super) fn photo_ids(&self) -> &[u64] {
        &self.photo_ids
    }
}

pub(super) struct CatalogCompatibility;

impl Compatibility for CatalogCompatibility {
    fn supports_snapshot(&self, encoding: &Encoding) -> bool {
        encoding.family == CATALOG_SNAPSHOT_FAMILY
            && encoding.version == VERSION
            && encoding.required_capabilities == [DEFLATE_CAPABILITY]
    }

    fn supports_operations(&self, encoding: &Encoding) -> bool {
        encoding.family == CATALOG_OPERATIONS_FAMILY
            && encoding.version <= VERSION
            && encoding.required_capabilities.is_empty()
    }
}

pub(super) struct PhotoCompatibility;

impl Compatibility for PhotoCompatibility {
    fn supports_snapshot(&self, encoding: &Encoding) -> bool {
        encoding.family == PHOTO_SNAPSHOT_FAMILY
            && encoding.version == VERSION
            && encoding.required_capabilities == [DEFLATE_CAPABILITY]
    }

    fn supports_operations(&self, encoding: &Encoding) -> bool {
        encoding.family == PHOTO_OPERATIONS_FAMILY
            && encoding.version <= VERSION
            && encoding.required_capabilities.is_empty()
    }
}

pub(super) struct LegacyCompatibility;

impl Compatibility for LegacyCompatibility {
    fn supports_snapshot(&self, encoding: &Encoding) -> bool {
        encoding.family == LEGACY_SNAPSHOT_FAMILY
            && encoding.version == VERSION
            && encoding.required_capabilities == [DEFLATE_CAPABILITY]
    }

    fn supports_operations(&self, encoding: &Encoding) -> bool {
        encoding.family == LEGACY_OPERATIONS_FAMILY
            && encoding.version <= VERSION
            && encoding.required_capabilities.is_empty()
    }
}

pub(super) fn catalog_snapshot(state: &CatalogState) -> Result<Payload> {
    snapshot(CATALOG_SNAPSHOT_FAMILY, state)
}

pub(super) fn catalog_operation(state: &CatalogState) -> Result<Payload> {
    operation(CATALOG_OPERATIONS_FAMILY, state)
}

pub(super) fn photo_snapshot(photo: &Photo, reference: &AssetReference) -> Result<Payload> {
    snapshot(PHOTO_SNAPSHOT_FAMILY, &portable_photo(photo, reference))
}

pub(super) fn photo_operation(photo: &Photo, reference: &AssetReference) -> Result<Payload> {
    operation(PHOTO_OPERATIONS_FAMILY, &portable_photo(photo, reference))
}

pub(super) fn decode_catalog_snapshot(payload: &Payload) -> Result<CatalogState> {
    decode_snapshot(payload)
}

pub(super) fn decode_catalog_operation(payload: &Payload) -> Result<CatalogState> {
    decode_operation(payload)
}

pub(super) fn decode_photo_snapshot(payload: &Payload) -> Result<Photo> {
    decode_snapshot(payload)
}

pub(super) fn decode_photo_operation(payload: &Payload) -> Result<Photo> {
    decode_operation(payload)
}

pub(super) fn decode_legacy_snapshot(payload: &Payload) -> Result<Project> {
    decode_snapshot(payload)
}

pub(super) fn photo_track_label(id: u64) -> String {
    format!("photo:{id}")
}

pub(super) fn photo_id_from_track_label(label: &str) -> Option<u64> {
    label.strip_prefix("photo:")?.parse().ok()
}

fn snapshot<T: Serialize>(family: &str, value: &T) -> Result<Payload> {
    Ok(Payload::new(
        Encoding::new(family, VERSION).requiring(DEFLATE_CAPABILITY),
        deflate(&serde_json::to_vec(value)?)?,
    ))
}

fn operation<T: Serialize>(family: &str, value: &T) -> Result<Payload> {
    Ok(Payload::new(
        Encoding::new(family, VERSION),
        serde_json::to_vec(value)?,
    ))
}

fn decode_snapshot<T: DeserializeOwned>(payload: &Payload) -> Result<T> {
    serde_json::from_slice(&inflate(&payload.bytes)?).context("invalid compressed Lumen snapshot")
}

fn decode_operation<T: DeserializeOwned>(payload: &Payload) -> Result<T> {
    serde_json::from_slice(&payload.bytes).context("invalid Lumen replacement operation")
}

fn portable_photo(photo: &Photo, reference: &AssetReference) -> Photo {
    let mut portable = photo.clone();
    portable.path = reference.path();
    portable
}

pub(super) struct PreparedPhoto {
    pub(super) reference: AssetReference,
    pub(super) asset: Asset,
}

impl PreparedPhoto {
    pub(super) fn new(photo: &Photo) -> Result<Self> {
        let reference = AssetReference::parse(&photo.path);
        if let Some(reference) = reference {
            bail!(
                "cannot prepare unresolved embedded asset reference {}",
                reference.id
            );
        }
        let bytes = fs::read(&photo.path)
            .with_context(|| format!("could not embed {}", photo.path.display()))?;
        let extension = photo
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(sanitize_extension)
            .filter(|extension| !extension.is_empty())
            .unwrap_or_else(|| "bin".into());
        let asset = Asset::new(media_type(&extension), bytes);
        Ok(Self {
            reference: AssetReference {
                id: asset.id,
                extension,
            },
            asset,
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct AssetReference {
    pub(super) id: AssetId,
    extension: String,
}

impl AssetReference {
    pub(super) fn parse(path: &Path) -> Option<Self> {
        let value = path.to_str()?.strip_prefix(ASSET_PREFIX)?;
        let (hash, extension) = value.split_once('.')?;
        let id = AssetId::from_hex(hash)?;
        let extension = sanitize_extension(extension);
        (!extension.is_empty()).then_some(Self { id, extension })
    }

    pub(super) fn path(&self) -> PathBuf {
        PathBuf::from(format!("{ASSET_PREFIX}{}.{}", self.id, self.extension))
    }

    pub(super) fn materialized_path(&self, project_id: &str) -> PathBuf {
        std::env::temp_dir()
            .join("spectrum-lumen-cache")
            .join(project_id)
            .join(format!("{}.{}", self.id, self.extension))
    }
}

fn deflate(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

fn inflate(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoded = Vec::new();
    ZlibDecoder::new(bytes).read_to_end(&mut decoded)?;
    Ok(decoded)
}

fn sanitize_extension(extension: &str) -> String {
    extension
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn media_type(extension: &str) -> &'static str {
    match extension {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "tif" | "tiff" => "image/tiff",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}
