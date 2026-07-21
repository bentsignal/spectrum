use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use image::{ImageDecoder, RgbaImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spectrum_imaging::{
    ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness, RegionRequestError,
    RegionSourceDescriptor, RegionSourceInfo, validate_region_request,
};

use crate::raster_region::{decoder_contract_for, inspect_raster_region_source};

const CACHE_SCHEMA_VERSION: u32 = 1;
const MANIFEST_FILE: &str = "manifest.json";
const PLANE_FILE: &str = "pixels.rgba8";
const READY_FILE: &str = "ready";
const PREPARE_LOCK: &str = ".prepare-lock";
const PIXEL_FORMAT: &str = "rgba8-straight-unpremultiplied";
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug)]
pub struct DerivedBackingLimits {
    pub max_dimension: u32,
    pub max_encoded_source_bytes: u64,
    pub max_plane_bytes: u64,
    pub max_cache_bytes: u64,
    pub max_region_pixels: u64,
}

impl Default for DerivedBackingLimits {
    fn default() -> Self {
        Self {
            // Preparation is intentionally a worker-only, full-decode operation.
            // The 2 GiB envelope admits an exact 16,384-square RGBA8 plane,
            // while the single builder prevents concurrent cold-decode spikes.
            max_dimension: crate::MAX_CANVAS_DIMENSION,
            max_encoded_source_bytes: 2 * 1_024 * 1_024 * 1_024,
            max_plane_bytes: 2 * 1_024 * 1_024 * 1_024,
            max_cache_bytes: 8 * 1_024 * 1_024 * 1_024,
            max_region_pixels: 4_096 * 4_096,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedBackingIdentity {
    key: String,
    source_sha256: String,
    descriptor: RegionSourceDescriptor,
}

impl DerivedBackingIdentity {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    pub fn descriptor(&self) -> &RegionSourceDescriptor {
        &self.descriptor
    }
}

pub enum PrepareDerivedBacking {
    Ready {
        backing: DerivedRasterBacking,
        created: bool,
    },
    InProgress(DerivedBackingIdentity),
}

pub struct DerivedBackingCache {
    root: PathBuf,
    limits: DerivedBackingLimits,
}

impl DerivedBackingCache {
    pub fn new(root: impl Into<PathBuf>, limits: DerivedBackingLimits) -> Self {
        Self {
            root: root.into(),
            limits,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Computes an immutable identity. Call this from an import/cache worker,
    /// never from a paint callback: hashing linked files is intentionally not a
    /// per-frame operation.
    pub fn identify(&self, source: &Path) -> Result<DerivedBackingIdentity> {
        let inspection = inspect_raster_region_source(source)?;
        let encoded_bytes = fs::metadata(source)
            .with_context(|| format!("could not inspect {}", source.display()))?
            .len();
        if encoded_bytes > self.limits.max_encoded_source_bytes {
            bail!("encoded raster exceeds the derived backing source byte limit");
        }
        if inspection.info.capability != RegionReadCapability::DerivedBacking
            || !inspection.info.descriptor.supports_exact_rgba8_backing()
        {
            bail!("raster source does not support an exact derived RGBA8 backing plane");
        }
        if inspection.info.descriptor.width > self.limits.max_dimension
            || inspection.info.descriptor.height > self.limits.max_dimension
        {
            bail!("raster dimensions exceed the derived backing dimension limit");
        }
        let source_sha256 = sha256_file(source)?;
        let confirmed = inspect_raster_region_source(source)?;
        if confirmed.info.capability != inspection.info.capability
            || confirmed.info.descriptor != inspection.info.descriptor
        {
            bail!("raster source changed while its cache identity was computed");
        }
        let key_material = CacheKeyMaterial {
            source_sha256: &source_sha256,
            descriptor: &inspection.info.descriptor,
        };
        let key = sha256_bytes(&serde_json::to_vec(&key_material)?);
        Ok(DerivedBackingIdentity {
            key,
            source_sha256,
            descriptor: inspection.info.descriptor,
        })
    }

    /// Fully validates a published plane before returning a retained provider.
    ///
    /// This includes hashing the complete plane. Registry/worker code must call
    /// it off the paint path and retain the returned provider across viewport
    /// reads rather than reopening it per frame.
    pub fn open_ready(
        &self,
        identity: &DerivedBackingIdentity,
    ) -> Result<Option<DerivedRasterBacking>> {
        let directory = self.entry_directory(identity);
        let ready_path = directory.join(READY_FILE);
        if !ready_path.is_file() {
            return Ok(None);
        }
        let manifest_path = directory.join(MANIFEST_FILE);
        let manifest_bytes = fs::read(&manifest_path)
            .with_context(|| format!("could not read {}", manifest_path.display()))?;
        let ready = fs::read_to_string(&ready_path)
            .with_context(|| format!("could not read {}", ready_path.display()))?;
        if ready.trim() != sha256_bytes(&manifest_bytes) {
            bail!("derived raster backing ready marker does not match its manifest");
        }
        let manifest: DerivedBackingManifest = serde_json::from_slice(&manifest_bytes)
            .context("derived raster backing manifest is invalid")?;
        manifest.validate(identity, self.limits)?;
        let plane_path = directory.join(PLANE_FILE);
        let plane_length = fs::metadata(&plane_path)
            .with_context(|| format!("could not inspect {}", plane_path.display()))?
            .len();
        if plane_length != manifest.plane_bytes {
            bail!("derived raster backing plane length does not match its manifest");
        }
        if sha256_file(&plane_path)? != manifest.plane_sha256 {
            bail!("derived raster backing plane checksum does not match its manifest");
        }
        Ok(Some(DerivedRasterBacking {
            info: RegionSourceInfo {
                descriptor: identity.descriptor.clone(),
                capability: RegionReadCapability::DerivedBacking,
                readiness: RegionReadiness::Ready,
            },
            key: identity.key.clone(),
            plane_path,
            max_region_pixels: self.limits.max_region_pixels,
        }))
    }

    /// Performs the one full decode needed to publish an immutable RGBA8 plane.
    /// A cache-wide OS file lock is the single-flight seam for the future
    /// worker coordinator, is released after crashes, and makes quota checks
    /// race-free.
    ///
    /// The cold path can transiently hold the decoder's native surface and the
    /// RGBA8 plane together. It must remain on the single background builder;
    /// this foundation deliberately does not invoke it from the GUI renderer.
    pub fn prepare(&self, source: &Path) -> Result<PrepareDerivedBacking> {
        let identity = self.identify(source)?;
        if let Ok(Some(backing)) = self.open_ready(&identity) {
            return Ok(PrepareDerivedBacking::Ready {
                backing,
                created: false,
            });
        }
        fs::create_dir_all(&self.root)
            .with_context(|| format!("could not create {}", self.root.display()))?;
        let Some(_lease) = CachePrepareLease::acquire(self.root.join(PREPARE_LOCK))? else {
            return Ok(PrepareDerivedBacking::InProgress(identity));
        };
        match self.open_ready(&identity) {
            Ok(Some(backing)) => {
                return Ok(PrepareDerivedBacking::Ready {
                    backing,
                    created: false,
                });
            }
            Ok(None) => {}
            Err(_) => {
                let corrupt = self.entry_directory(&identity);
                remove_cache_entry(&corrupt).with_context(|| {
                    format!(
                        "could not discard corrupt cache entry {}",
                        corrupt.display()
                    )
                })?;
            }
        }

        let plane_bytes = identity
            .descriptor
            .exact_rgba8_plane_bytes()
            .context("derived raster backing dimensions overflow")?;
        if plane_bytes > self.limits.max_plane_bytes {
            bail!("derived raster backing exceeds the per-source byte limit");
        }
        let occupied = self.occupied_plane_bytes()?;
        if occupied
            .checked_add(plane_bytes)
            .is_none_or(|total| total > self.limits.max_cache_bytes)
        {
            bail!("derived raster backing cache quota would be exceeded");
        }

        let pixels = decode_exact_rgba8(source, &identity, self.limits)?;
        let raw = pixels.into_raw();
        if u64::try_from(raw.len()).ok() != Some(plane_bytes) {
            bail!("decoded raster byte count does not match its backing descriptor");
        }
        let manifest = DerivedBackingManifest {
            schema_version: CACHE_SCHEMA_VERSION,
            key: identity.key.clone(),
            source_sha256: identity.source_sha256.clone(),
            descriptor: identity.descriptor.clone(),
            pixel_format: PIXEL_FORMAT.into(),
            row_stride: u64::from(identity.descriptor.width) * 4,
            plane_bytes,
            plane_sha256: sha256_bytes(&raw),
        };
        self.publish(&identity, &manifest, &raw)?;
        let backing = self
            .open_ready(&identity)?
            .context("published derived raster backing is not ready")?;
        Ok(PrepareDerivedBacking::Ready {
            backing,
            created: true,
        })
    }

    fn publish(
        &self,
        identity: &DerivedBackingIdentity,
        manifest: &DerivedBackingManifest,
        raw: &[u8],
    ) -> Result<()> {
        let temporary_name = format!(
            ".tmp-{}-{}-{}",
            identity.key,
            std::process::id(),
            TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let temporary = TemporaryDirectory::create(self.root.join(temporary_name))?;
        write_immutable_file(&temporary.path.join(PLANE_FILE), raw)?;
        let manifest_bytes = serde_json::to_vec(manifest)?;
        write_immutable_file(&temporary.path.join(MANIFEST_FILE), &manifest_bytes)?;
        write_immutable_file(
            &temporary.path.join(READY_FILE),
            format!("{}\n", sha256_bytes(&manifest_bytes)).as_bytes(),
        )?;
        sync_directory(&temporary.path)?;

        let destination = self.entry_directory(identity);
        if destination.exists() {
            remove_cache_entry(&destination).with_context(|| {
                format!(
                    "could not discard corrupt cache entry {}",
                    destination.display()
                )
            })?;
        }
        fs::rename(&temporary.path, &destination).with_context(|| {
            format!(
                "could not atomically publish derived raster backing {}",
                destination.display()
            )
        })?;
        sync_directory(&self.root)?;
        temporary.commit();
        Ok(())
    }

    fn occupied_plane_bytes(&self) -> Result<u64> {
        let mut occupied = 0_u64;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Ok(metadata) = fs::metadata(entry.path().join(PLANE_FILE)) {
                occupied = occupied
                    .checked_add(metadata.len())
                    .context("derived raster backing cache size overflows")?;
            }
        }
        Ok(occupied)
    }

    fn entry_directory(&self, identity: &DerivedBackingIdentity) -> PathBuf {
        self.root.join(&identity.key)
    }
}

pub struct DerivedRasterBacking {
    info: RegionSourceInfo,
    key: String,
    plane_path: PathBuf,
    max_region_pixels: u64,
}

impl DerivedRasterBacking {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn plane_path(&self) -> &Path {
        &self.plane_path
    }
}

impl ExactRegionSource for DerivedRasterBacking {
    type Error = DerivedBackingReadError;

    fn info(&self) -> &RegionSourceInfo {
        &self.info
    }

    fn read_exact_region(&self, region: PixelRegion) -> Result<RgbaImage, Self::Error> {
        validate_region_request(&self.info.descriptor, region, self.max_region_pixels)
            .map_err(DerivedBackingReadError::Request)?;
        let row_bytes = usize::try_from(u64::from(region.width) * 4)
            .map_err(|_| DerivedBackingReadError::LayoutOverflow)?;
        let output_bytes = row_bytes
            .checked_mul(region.height as usize)
            .ok_or(DerivedBackingReadError::LayoutOverflow)?;
        let mut output = vec![0; output_bytes];
        let source_stride = u64::from(self.info.descriptor.width) * 4;
        let mut plane = BufReader::new(File::open(&self.plane_path)?);
        for row in 0..region.height {
            let offset = u64::from(region.y + row)
                .checked_mul(source_stride)
                .and_then(|offset| offset.checked_add(u64::from(region.x) * 4))
                .ok_or(DerivedBackingReadError::LayoutOverflow)?;
            plane.seek(SeekFrom::Start(offset))?;
            let start = row as usize * row_bytes;
            plane.read_exact(&mut output[start..start + row_bytes])?;
        }
        RgbaImage::from_raw(region.width, region.height, output)
            .ok_or(DerivedBackingReadError::InvalidPlane)
    }
}

#[derive(Debug)]
pub enum DerivedBackingReadError {
    Request(RegionRequestError),
    Io(io::Error),
    LayoutOverflow,
    InvalidPlane,
}

impl std::fmt::Display for DerivedBackingReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
            Self::LayoutOverflow => formatter.write_str("derived backing layout overflows"),
            Self::InvalidPlane => formatter.write_str("derived backing plane is invalid"),
        }
    }
}

impl std::error::Error for DerivedBackingReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Request(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::LayoutOverflow | Self::InvalidPlane => None,
        }
    }
}

impl From<io::Error> for DerivedBackingReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Serialize)]
struct CacheKeyMaterial<'a> {
    source_sha256: &'a str,
    descriptor: &'a RegionSourceDescriptor,
}

#[derive(Deserialize, Serialize)]
struct DerivedBackingManifest {
    schema_version: u32,
    key: String,
    source_sha256: String,
    descriptor: RegionSourceDescriptor,
    pixel_format: String,
    row_stride: u64,
    plane_bytes: u64,
    plane_sha256: String,
}

impl DerivedBackingManifest {
    fn validate(
        &self,
        identity: &DerivedBackingIdentity,
        limits: DerivedBackingLimits,
    ) -> Result<()> {
        if self.schema_version != CACHE_SCHEMA_VERSION
            || self.key != identity.key
            || self.source_sha256 != identity.source_sha256
            || self.descriptor != identity.descriptor
        {
            bail!("derived raster backing manifest does not match its source identity");
        }
        if self.pixel_format != PIXEL_FORMAT
            || self.row_stride != u64::from(self.descriptor.width) * 4
        {
            bail!("derived raster backing manifest has an unsupported pixel layout");
        }
        if self.descriptor.width > limits.max_dimension
            || self.descriptor.height > limits.max_dimension
        {
            bail!("derived raster backing manifest exceeds its dimension contract");
        }
        let expected = self
            .descriptor
            .exact_rgba8_plane_bytes()
            .context("derived raster backing dimensions overflow")?;
        if self.plane_bytes != expected || self.plane_bytes > limits.max_plane_bytes {
            bail!("derived raster backing manifest exceeds its byte contract");
        }
        Ok(())
    }
}

fn decode_exact_rgba8(
    source: &Path,
    identity: &DerivedBackingIdentity,
    limits: DerivedBackingLimits,
) -> Result<RgbaImage> {
    let mut source_file =
        File::open(source).with_context(|| format!("could not open {}", source.display()))?;
    if source_file.metadata()?.len() > limits.max_encoded_source_bytes {
        bail!("encoded raster exceeds the derived backing source byte limit");
    }
    let before_hash = sha256_reader(&mut source_file)?;
    if before_hash != identity.source_sha256 {
        bail!("raster source changed before its derived backing was prepared");
    }
    source_file.seek(SeekFrom::Start(0))?;
    let reader = image::ImageReader::new(BufReader::new(&mut source_file))
        .with_guessed_format()
        .with_context(|| format!("could not identify {}", source.display()))?;
    let format = reader
        .format()
        .with_context(|| format!("could not identify {}", source.display()))?;
    if decoder_contract_for(format) != identity.descriptor.decoder_contract {
        bail!("raster decoder contract changed before backing preparation");
    }
    let mut image_limits = image::Limits::default();
    image_limits.max_image_width = Some(identity.descriptor.width);
    image_limits.max_image_height = Some(identity.descriptor.height);
    image_limits.max_alloc = Some(limits.max_plane_bytes);
    let mut decoder = reader
        .into_decoder()
        .with_context(|| format!("could not inspect {}", source.display()))?;
    if decoder.dimensions() != (identity.descriptor.width, identity.descriptor.height)
        || format!("{:?}", decoder.color_type()).to_ascii_lowercase()
            != identity.descriptor.color_encoding
    {
        bail!("raster decoder metadata changed before backing preparation");
    }
    decoder.set_limits(image_limits)?;
    let decoded = image::DynamicImage::from_decoder(decoder)
        .with_context(|| format!("could not decode {}", source.display()))?
        .to_rgba8();
    source_file.seek(SeekFrom::Start(0))?;
    if sha256_reader(&mut source_file)? != identity.source_sha256 {
        bail!("raster source changed while its derived backing was prepared");
    }
    if decoded.dimensions() != (identity.descriptor.width, identity.descriptor.height) {
        bail!("raster dimensions changed while its derived backing was prepared");
    }
    Ok(decoded)
}

fn write_immutable_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    let mut permissions = file.metadata()?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn remove_cache_entry(path: &Path) -> io::Result<()> {
    make_cache_tree_removable(path)?;
    fs::remove_dir_all(path)
}

#[cfg(not(windows))]
fn make_cache_tree_removable(path: &Path) -> io::Result<()> {
    fs::metadata(path).map(|_| ())
}

#[cfg(windows)]
fn make_cache_tree_removable(path: &Path) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            make_cache_tree_removable(&entry.path())?;
        } else {
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            fs::set_permissions(entry.path(), permissions)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = BufReader::new(
        File::open(path).with_context(|| format!("could not open {}", path.display()))?,
    );
    sha256_reader(&mut file)
}

fn sha256_reader(reader: &mut impl Read) -> Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 128 * 1_024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct CachePrepareLease {
    _file: File,
}

impl CachePrepareLease {
    fn acquire(path: PathBuf) -> Result<Option<Self>> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

struct TemporaryDirectory {
    path: PathBuf,
    committed: bool,
}

impl TemporaryDirectory {
    fn create(path: PathBuf) -> Result<Self> {
        fs::create_dir(&path).with_context(|| format!("could not create {}", path.display()))?;
        Ok(Self {
            path,
            committed: false,
        })
    }

    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
