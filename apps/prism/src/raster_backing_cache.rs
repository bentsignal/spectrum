use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spectrum_imaging::{
    ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness, RegionRequestError,
    RegionSourceDescriptor, RegionSourceInfo, validate_region_request,
};

use crate::raster_region::{decoder_contract_for, inspect_raster_region_source};

mod cache_fs;
mod maintenance;
pub(crate) mod prepare;
use cache_fs::{
    RetainedPlane, open_trusted_cache_file, read_bounded, read_exact_at, remove_cache_entry,
    retain_plane, sync_directory, trusted_cache_directory, trusted_cache_directory_if_present,
};
use maintenance::{CacheMaintenanceLease, EntryReadLease};
use prepare::prepare_exact_rgba8_plane;

const CACHE_SCHEMA_VERSION: u32 = 2;
const LEGACY_CACHE_SCHEMA_VERSION: u32 = 1;
const CACHE_VERSION_DIRECTORY: &str = "v2";
const MANIFEST_FILE: &str = "manifest.json";
const PLANE_FILE: &str = "pixels.rgba8";
const READY_FILE: &str = "ready";
const ENTRY_LEASE_FILE: &str = ".lease";
const ACCESS_FILE: &str = ".access";
const ACCESS_MARKER_BYTES: u64 = 1;
const PREPARE_LOCK: &str = ".prepare-lock";
const PIXEL_FORMAT: &str = "rgba8-straight-unpremultiplied";
const MAX_MANIFEST_BYTES: u64 = 64 * 1_024;
const MAX_READY_BYTES: u64 = 128;
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
        memory_plan: DerivedBackingMemoryPlan,
    },
    InProgress(DerivedBackingIdentity),
}

/// Accounted pixel storage for one cold derived-backing preparation.
///
/// Prism publication retains one decoded output surface and writes it directly,
/// or converts one row at a time for RGB/L/LA inputs. Decoder dependencies can
/// own additional full-frame or input buffers while producing that surface.
/// Those known reservations are reported separately; because dependency-private
/// allocations are not fully limited by `image`, this plan does not advertise
/// an enforced total-process peak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DerivedBackingMemoryPlan {
    decoded_surface_bytes: u64,
    conversion_row_bytes: u64,
    decoder_scratch_reservation_bytes: u64,
    encoded_input_reservation_bytes: u64,
    known_resident_reservation_bytes: u64,
}

impl DerivedBackingMemoryPlan {
    pub fn decoded_surface_bytes(self) -> u64 {
        self.decoded_surface_bytes
    }

    pub fn conversion_row_bytes(self) -> u64 {
        self.conversion_row_bytes
    }

    /// Conservative dependency pixel-scratch reservation concurrent with the
    /// output surface. WebP reserves one RGBA plane because opaque lossless
    /// decode requires it; some layouts may use less or additional private work.
    pub fn decoder_scratch_reservation_bytes(self) -> u64 {
        self.decoder_scratch_reservation_bytes
    }

    /// Conservative encoded-input reservation for decoders that buffer input.
    pub fn encoded_input_reservation_bytes(self) -> u64 {
        self.encoded_input_reservation_bytes
    }

    /// Reservation covering all known concurrent buffers in the pinned decoder
    /// implementations. This is not an upper bound on dependency-private work.
    pub fn known_resident_reservation_bytes(self) -> u64 {
        self.known_resident_reservation_bytes
    }

    /// `image` does not enforce a complete bound on dependency-private decoder
    /// allocations, so supported derived formats currently return `None`.
    pub fn enforced_peak_bytes(self) -> Option<u64> {
        None
    }

    /// Publication never allocates a second full RGBA8 plane in memory.
    pub fn full_plane_copy_bytes(self) -> u64 {
        0
    }
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
        let source_sha256 = sha256_path_bounded(
            source,
            self.limits.max_encoded_source_bytes,
            "encoded raster",
        )?;
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
        self.validate_identity(identity)?;
        if !trusted_cache_directory_if_present(&self.root)? {
            return Ok(None);
        }
        let version_root = self.version_root();
        if !trusted_cache_directory_if_present(&version_root)? {
            return Ok(None);
        }
        let directory = self.entry_directory(identity);
        if !trusted_cache_directory_if_present(&directory)? {
            return Ok(None);
        }
        let entry_lease = EntryReadLease::acquire(&directory)?;
        let ready_path = directory.join(READY_FILE);
        match fs::symlink_metadata(&ready_path) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        let manifest_path = directory.join(MANIFEST_FILE);
        let mut manifest_file = open_trusted_cache_file(&manifest_path, MAX_MANIFEST_BYTES)?;
        let manifest_bytes = read_bounded(&mut manifest_file, MAX_MANIFEST_BYTES, "manifest")?;
        let mut ready_file = open_trusted_cache_file(&ready_path, MAX_READY_BYTES)?;
        let ready = String::from_utf8(read_bounded(
            &mut ready_file,
            MAX_READY_BYTES,
            "ready marker",
        )?)
        .context("derived raster backing ready marker is not UTF-8")?;
        if ready.trim() != sha256_bytes(&manifest_bytes) {
            bail!("derived raster backing ready marker does not match its manifest");
        }
        let manifest: DerivedBackingManifest = serde_json::from_slice(&manifest_bytes)
            .context("derived raster backing manifest is invalid")?;
        manifest.validate(identity, self.limits)?;
        let plane_path = directory.join(PLANE_FILE);
        let mut plane = open_trusted_cache_file(&plane_path, self.limits.max_plane_bytes)?;
        let plane_length = plane.metadata()?.len();
        if plane_length != manifest.plane_bytes {
            bail!("derived raster backing plane length does not match its manifest");
        }
        if sha256_reader_bounded(&mut plane, self.limits.max_plane_bytes, "backing plane")?
            != manifest.plane_sha256
        {
            bail!("derived raster backing plane checksum does not match its manifest");
        }
        plane.seek(SeekFrom::Start(0))?;
        Ok(Some(DerivedRasterBacking {
            info: RegionSourceInfo {
                descriptor: identity.descriptor.clone(),
                capability: RegionReadCapability::DerivedBacking,
                readiness: RegionReadiness::Ready,
            },
            key: identity.key.clone(),
            plane_path,
            plane: retain_plane(plane),
            _entry_lease: entry_lease,
            max_region_pixels: self.limits.max_region_pixels,
        }))
    }

    /// Performs the one full decode needed to publish an immutable RGBA8 plane.
    /// A cache-wide OS file lock is the single-flight seam for the future
    /// worker coordinator, is released after crashes, and makes quota checks
    /// race-free.
    ///
    /// Prism's publication path holds one decoded output surface and at most one
    /// RGBA8 conversion row. Decoder-private memory is serialized globally and
    /// accounted conservatively where dependency behavior is known.
    pub fn prepare(&self, source: &Path) -> Result<PrepareDerivedBacking> {
        let identity = self.identify(source)?;
        self.prepare_identified(source, &identity)
    }

    /// Prepares a source using an identity retained by a worker retry loop.
    ///
    /// Busy retries validate the immutable identity but do not rehash or
    /// reinspect the source. Once this call owns the build lease, the decode
    /// path validates the source against the identity before and after its
    /// single decode.
    pub fn prepare_identified(
        &self,
        source: &Path,
        identity: &DerivedBackingIdentity,
    ) -> Result<PrepareDerivedBacking> {
        let memory_plan = self.validate_identity(identity)?;
        if let Ok(Some(backing)) = self.open_ready(identity) {
            return Ok(PrepareDerivedBacking::Ready {
                backing,
                created: false,
                memory_plan,
            });
        }
        self.ensure_cache_root()?;
        let version_root = self.version_root();
        let Some(maintenance) =
            CacheMaintenanceLease::try_acquire(&self.root, &version_root, self.limits)?
        else {
            return Ok(PrepareDerivedBacking::InProgress(identity.clone()));
        };
        maintenance.ensure_version_root()?;
        maintenance.scavenge_crash_entries()?;
        match self.open_ready(identity) {
            Ok(Some(backing)) => {
                return Ok(PrepareDerivedBacking::Ready {
                    backing,
                    created: false,
                    memory_plan,
                });
            }
            Ok(None) => {}
            Err(_) => {
                maintenance.remove_corrupt_entry(identity.key())?;
            }
        }

        let plane_bytes = identity
            .descriptor
            .exact_rgba8_plane_bytes()
            .context("derived raster backing dimensions overflow")?;
        if plane_bytes > self.limits.max_plane_bytes {
            bail!("derived raster backing exceeds the per-source byte limit");
        }
        let temporary_name = format!(
            ".tmp-{}-{}-{}",
            identity.key,
            std::process::id(),
            TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let temporary = TemporaryDirectory::create(version_root.join(temporary_name))?;
        let prepared = prepare_exact_rgba8_plane(
            source,
            identity,
            self.limits,
            &temporary.path.join(PLANE_FILE),
        )?;
        if prepared.plane_bytes != plane_bytes || prepared.memory_plan != memory_plan {
            bail!("decoded raster does not match its preparation memory contract");
        }
        let manifest = DerivedBackingManifest {
            schema_version: CACHE_SCHEMA_VERSION,
            key: identity.key.clone(),
            source_sha256: identity.source_sha256.clone(),
            descriptor: identity.descriptor.clone(),
            pixel_format: PIXEL_FORMAT.into(),
            row_stride: u64::from(identity.descriptor.width) * 4,
            plane_bytes,
            plane_sha256: prepared.plane_sha256,
        };
        self.publish(identity, &manifest, temporary, &maintenance)?;
        let backing = self
            .open_ready(identity)?
            .context("published derived raster backing is not ready")?;
        Ok(PrepareDerivedBacking::Ready {
            backing,
            created: true,
            memory_plan,
        })
    }

    fn publish(
        &self,
        identity: &DerivedBackingIdentity,
        manifest: &DerivedBackingManifest,
        temporary: TemporaryDirectory,
        maintenance: &CacheMaintenanceLease,
    ) -> Result<()> {
        let manifest_bytes = serde_json::to_vec(manifest)?;
        write_immutable_file(&temporary.path.join(MANIFEST_FILE), &manifest_bytes)?;
        write_immutable_file(
            &temporary.path.join(READY_FILE),
            format!("{}\n", sha256_bytes(&manifest_bytes)).as_bytes(),
        )?;
        write_mutable_file(&temporary.path.join(ENTRY_LEASE_FILE), &[])?;
        write_mutable_file(&temporary.path.join(ACCESS_FILE), b"0")?;
        sync_directory(&temporary.path)?;
        let incoming = maintenance.staged_logical_bytes(&temporary.path, identity.key())?;
        maintenance.ensure_quota(incoming, identity.key())?;

        let destination = self.entry_directory(identity);
        if fs::symlink_metadata(&destination).is_ok() {
            bail!("derived backing destination appeared during publication");
        }
        fs::rename(&temporary.path, &destination).with_context(|| {
            format!(
                "could not atomically publish derived raster backing {}",
                destination.display()
            )
        })?;
        sync_directory(&self.version_root())?;
        temporary.commit();
        Ok(())
    }

    /// Returns the pixel-storage peak enforced by cold preparation.
    pub fn preparation_memory_plan(
        &self,
        identity: &DerivedBackingIdentity,
    ) -> Result<DerivedBackingMemoryPlan> {
        self.validate_identity(identity)
    }

    fn validate_identity(
        &self,
        identity: &DerivedBackingIdentity,
    ) -> Result<DerivedBackingMemoryPlan> {
        if !is_lower_sha256(&identity.source_sha256)
            || !identity.descriptor.supports_exact_rgba8_backing()
        {
            bail!("derived raster backing identity is invalid");
        }
        if identity.descriptor.width > self.limits.max_dimension
            || identity.descriptor.height > self.limits.max_dimension
        {
            bail!("raster dimensions exceed the derived backing dimension limit");
        }
        let expected_key = sha256_bytes(&serde_json::to_vec(&CacheKeyMaterial {
            source_sha256: &identity.source_sha256,
            descriptor: &identity.descriptor,
        })?);
        if identity.key != expected_key {
            bail!("derived raster backing identity key is invalid");
        }
        let plan = prepare::memory_plan(&identity.descriptor, self.limits)?;
        let plane_bytes = identity
            .descriptor
            .exact_rgba8_plane_bytes()
            .context("derived raster backing dimensions overflow")?;
        if plane_bytes > self.limits.max_plane_bytes {
            bail!("derived raster backing exceeds the per-source byte limit");
        }
        Ok(plan)
    }

    fn ensure_cache_root(&self) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("could not create {}", self.root.display()))?;
        trusted_cache_directory(&self.root)
    }

    fn version_root(&self) -> PathBuf {
        self.root.join(CACHE_VERSION_DIRECTORY)
    }

    fn entry_directory(&self, identity: &DerivedBackingIdentity) -> PathBuf {
        self.version_root().join(&identity.key)
    }
}

pub struct DerivedRasterBacking {
    info: RegionSourceInfo,
    key: String,
    plane_path: PathBuf,
    plane: RetainedPlane,
    _entry_lease: EntryReadLease,
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
        for row in 0..region.height {
            let offset = u64::from(region.y + row)
                .checked_mul(source_stride)
                .and_then(|offset| offset.checked_add(u64::from(region.x) * 4))
                .ok_or(DerivedBackingReadError::LayoutOverflow)?;
            let start = row as usize * row_bytes;
            read_exact_at(&self.plane, &mut output[start..start + row_bytes], offset)?;
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
        self.validate_schema(identity, limits, CACHE_SCHEMA_VERSION)
    }

    fn validate_schema(
        &self,
        identity: &DerivedBackingIdentity,
        limits: DerivedBackingLimits,
        expected_schema: u32,
    ) -> Result<()> {
        if self.schema_version != expected_schema
            || self.key != identity.key
            || self.source_sha256 != identity.source_sha256
            || self.descriptor != identity.descriptor
        {
            bail!("derived raster backing manifest does not match its source identity");
        }
        if !is_lower_sha256(&self.plane_sha256)
            || self.pixel_format != PIXEL_FORMAT
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

fn write_immutable_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    let mut permissions = file.metadata()?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn write_mutable_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn validate_inventory_manifest(
    manifest: &DerivedBackingManifest,
    key: &str,
    limits: DerivedBackingLimits,
    expected_schema: u32,
) -> Result<()> {
    if manifest.key != key || !is_lower_sha256(&manifest.source_sha256) {
        bail!("derived backing manifest has an invalid content identity");
    }
    let expected_key = sha256_bytes(&serde_json::to_vec(&CacheKeyMaterial {
        source_sha256: &manifest.source_sha256,
        descriptor: &manifest.descriptor,
    })?);
    if expected_key != key {
        bail!("derived backing manifest content address is invalid");
    }
    let identity = DerivedBackingIdentity {
        key: manifest.key.clone(),
        source_sha256: manifest.source_sha256.clone(),
        descriptor: manifest.descriptor.clone(),
    };
    manifest.validate_schema(&identity, limits, expected_schema)
}

fn sha256_path_bounded(path: &Path, max_bytes: u64, label: &str) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    if !file.metadata()?.is_file() {
        bail!("{label} is not a regular file");
    }
    sha256_reader_bounded(&mut file, max_bytes, label)
}

fn sha256_reader_bounded(reader: &mut impl Read, max_bytes: u64, label: &str) -> Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 128 * 1_024];
    let mut total = 0_u64;
    loop {
        let remaining = max_bytes.saturating_sub(total);
        let requested = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        if requested == 0 {
            let mut extra = [0_u8; 1];
            if reader.read(&mut extra)? != 0 {
                bail!("{label} exceeds its byte limit");
            }
            break;
        }
        let read = reader.read(&mut buffer[..requested])?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        total = total
            .checked_add(read as u64)
            .context("hashed byte count overflows")?;
    }
    Ok(sha256_hex(digest.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    sha256_hex(Sha256::digest(bytes))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_eviction_tombstone_name(value: &str) -> bool {
    let Some(value) = value.strip_prefix(".evict-") else {
        return false;
    };
    let mut fields = value.split('-');
    let (Some(key), Some(process), Some(counter), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    is_lower_sha256(key)
        && !process.is_empty()
        && process.bytes().all(|byte| byte.is_ascii_digit())
        && !counter.is_empty()
        && counter.bytes().all(|byte| byte.is_ascii_digit())
}

fn sha256_hex(digest: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let digest = digest.as_ref();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

struct TemporaryDirectory {
    path: PathBuf,
    committed: bool,
}

impl TemporaryDirectory {
    fn create(path: PathBuf) -> Result<Self> {
        fs::create_dir(&path).with_context(|| format!("could not create {}", path.display()))?;
        trusted_cache_directory(&path)?;
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
            let _ = remove_cache_entry(&self.path);
        }
    }
}
