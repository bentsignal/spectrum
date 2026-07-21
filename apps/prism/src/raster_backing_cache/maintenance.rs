use std::{
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use anyhow::{Context, Result, bail};
use fs2::FileExt;

use super::cache_fs::{
    is_cache_key, is_temporary_entry_name, open_or_create_trusted_lock_file,
    open_trusted_cache_file, open_trusted_cache_file_rw, path_refers_to_file, read_bounded,
    remove_cache_entry, sync_directory, trusted_cache_directory,
    trusted_cache_directory_if_present,
};
use super::{
    ACCESS_FILE, ACCESS_MARKER_BYTES, DerivedBackingLimits, ENTRY_LEASE_FILE,
    LEGACY_CACHE_SCHEMA_VERSION, MANIFEST_FILE, MAX_MANIFEST_BYTES, MAX_READY_BYTES, PLANE_FILE,
    PREPARE_LOCK, READY_FILE, is_eviction_tombstone_name, is_lower_sha256, sha256_bytes,
};

static EVICTION_COUNTER: AtomicU64 = AtomicU64::new(1);
static ACCESS_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(super) struct EntryReadLease {
    _file: File,
}

impl EntryReadLease {
    pub fn acquire(entry: &Path) -> Result<Self> {
        trusted_cache_directory(entry)?;
        let path = entry.join(ENTRY_LEASE_FILE);
        let file = open_trusted_cache_file_rw(&path, 0)?;
        FileExt::lock_shared(&file)?;
        if !trusted_cache_directory_if_present(entry)? || !path_refers_to_file(&path, &file)? {
            bail!("derived backing entry changed while its read lease was acquired");
        }
        touch_access(&entry.join(ACCESS_FILE))?;
        Ok(Self { _file: file })
    }
}

/// Serializes v2 mutation and observes legacy entries under the root lock.
///
/// This coordinates with a v1 writer that started first. It cannot make a v1
/// binary quota-aware of an already-active `v2` directory, so callers must not
/// share a cache root across concurrent versions or downgrade onto a v2 root.
pub(super) struct CacheMaintenanceLease {
    root: PathBuf,
    version_root: PathBuf,
    _file: File,
    limits: DerivedBackingLimits,
}

impl CacheMaintenanceLease {
    pub fn try_acquire(
        root: &Path,
        version_root: &Path,
        limits: DerivedBackingLimits,
    ) -> Result<Option<Self>> {
        trusted_cache_directory(root)?;
        let path = root.join(PREPARE_LOCK);
        let file = open_or_create_trusted_lock_file(&path)?;
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(Some(Self {
                root: root.to_owned(),
                version_root: version_root.to_owned(),
                _file: file,
                limits,
            })),
            Err(error) if error.kind() == fs2::lock_contended_error().kind() => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn ensure_version_root(&self) -> Result<()> {
        match fs::create_dir(&self.version_root) {
            Ok(()) => sync_directory(&self.root)?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        trusted_cache_directory(&self.version_root)
    }

    pub fn scavenge_crash_entries(&self) -> Result<()> {
        self.scavenge_directory(&self.root, false)?;
        self.scavenge_directory(&self.version_root, true)
    }

    fn scavenge_directory(&self, directory: &Path, include_evictions: bool) -> Result<()> {
        let mut changed = false;
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if is_temporary_entry_name(&name)
                || (include_evictions && is_eviction_tombstone_name(&name))
            {
                remove_cache_entry(&entry.path()).with_context(|| {
                    format!(
                        "could not scavenge stale cache entry {}",
                        entry.path().display()
                    )
                })?;
                changed = true;
            }
        }
        if changed {
            sync_directory(directory)?;
        }
        Ok(())
    }

    pub fn remove_corrupt_entry(&self, key: &str) -> Result<()> {
        let path = self.version_root.join(key);
        if !trusted_cache_directory_if_present(&path)? {
            return Ok(());
        }
        match try_entry_exclusive_lease(&path)? {
            EntryExclusiveLease::Contended => {
                bail!("corrupt derived backing entry is retained by an active reader")
            }
            EntryExclusiveLease::Missing => self.tombstone_and_remove(&path, key, None),
            EntryExclusiveLease::Held(file) => self.tombstone_and_remove(&path, key, Some(file)),
        }
    }

    pub fn staged_logical_bytes(&self, path: &Path, key: &str) -> Result<u64> {
        Ok(inventory_complete_entry(path, key, self.limits)?.logical_bytes)
    }

    pub fn ensure_quota(&self, incoming_bytes: u64, protected_key: &str) -> Result<()> {
        if incoming_bytes > self.limits.max_cache_bytes {
            bail!("derived raster backing exceeds the total cache quota");
        }
        let mut entries = self.inventory_v2()?;
        let v2_bytes = entries.iter().try_fold(0_u64, |total, entry| {
            total
                .checked_add(entry.logical_bytes)
                .context("derived raster backing cache size overflows")
        })?;
        let legacy_bytes = self.inventory_legacy_bytes()?;
        let mut occupied = legacy_bytes
            .checked_add(v2_bytes)
            .context("cross-version derived backing cache size overflows")?;
        if occupied
            .checked_add(incoming_bytes)
            .is_some_and(|total| total <= self.limits.max_cache_bytes)
        {
            return Ok(());
        }
        entries.sort_by(|left, right| {
            left.last_access
                .cmp(&right.last_access)
                .then_with(|| left.key.cmp(&right.key))
        });
        for entry in entries {
            if entry.key == protected_key {
                continue;
            }
            let lease = match try_entry_exclusive_lease(&entry.path)? {
                EntryExclusiveLease::Held(file) => file,
                EntryExclusiveLease::Contended => continue,
                EntryExclusiveLease::Missing => {
                    bail!("complete derived backing entry lost its lease file")
                }
            };
            let confirmed = inventory_complete_entry(&entry.path, &entry.key, self.limits)?;
            if confirmed.logical_bytes != entry.logical_bytes {
                bail!("derived backing entry changed during quota inventory");
            }
            if confirmed.last_access != entry.last_access {
                drop(lease);
                continue;
            }
            self.tombstone_and_remove(&entry.path, &entry.key, Some(lease))?;
            occupied = occupied
                .checked_sub(entry.logical_bytes)
                .context("derived raster backing cache accounting underflows")?;
            if occupied
                .checked_add(incoming_bytes)
                .is_some_and(|total| total <= self.limits.max_cache_bytes)
            {
                return Ok(());
            }
        }
        bail!("derived raster backing cache quota cannot evict active or legacy entries")
    }

    fn inventory_v2(&self) -> Result<Vec<CacheEntryInventory>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.version_root)? {
            let entry = entry?;
            let Some(key) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if is_cache_key(&key) {
                entries.push(inventory_complete_entry(&entry.path(), &key, self.limits)?);
            }
        }
        Ok(entries)
    }

    fn inventory_legacy_bytes(&self) -> Result<u64> {
        fs::read_dir(&self.root)?.try_fold(0_u64, |total, entry| {
            let entry = entry?;
            let Some(key) = entry.file_name().to_str().map(str::to_owned) else {
                return Ok(total);
            };
            if !is_cache_key(&key) {
                return Ok(total);
            }
            let bytes = inventory_legacy_entry(&entry.path(), &key, self.limits)?;
            total
                .checked_add(bytes)
                .context("legacy derived backing cache size overflows")
        })
    }

    fn tombstone_and_remove(&self, path: &Path, key: &str, lease: Option<File>) -> Result<()> {
        let tombstone = self.version_root.join(format!(
            ".evict-{key}-{}-{}",
            std::process::id(),
            EVICTION_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::rename(path, &tombstone).with_context(|| {
            format!(
                "could not tombstone derived backing entry {}",
                path.display()
            )
        })?;
        sync_directory(&self.version_root)?;
        // Windows cannot recursively delete an entry while its lease handle is
        // retained. The durable rename happens first; only then release it.
        drop(lease);
        remove_cache_entry(&tombstone).with_context(|| {
            format!(
                "could not remove evicted cache entry {}",
                tombstone.display()
            )
        })?;
        sync_directory(&self.version_root)?;
        Ok(())
    }
}

struct CacheEntryInventory {
    key: String,
    path: PathBuf,
    logical_bytes: u64,
    last_access: SystemTime,
}

fn inventory_complete_entry(
    path: &Path,
    key: &str,
    limits: DerivedBackingLimits,
) -> Result<CacheEntryInventory> {
    if !is_lower_sha256(key) || !trusted_cache_directory_if_present(path)? {
        bail!("derived backing inventory encountered an invalid entry directory");
    }
    let mut seen = [false; 5];
    for child in fs::read_dir(path)? {
        let child = child?;
        let Some(name) = child.file_name().to_str().map(str::to_owned) else {
            bail!("derived backing entry contains a non-UTF-8 file name");
        };
        let index = match name.as_str() {
            MANIFEST_FILE => 0,
            PLANE_FILE => 1,
            READY_FILE => 2,
            ENTRY_LEASE_FILE => 3,
            ACCESS_FILE => 4,
            _ => bail!("derived backing entry contains an unexpected file {name}"),
        };
        if std::mem::replace(&mut seen[index], true) {
            bail!("derived backing entry contains a duplicate file {name}");
        }
    }
    if seen.iter().any(|present| !present) {
        bail!("derived backing inventory encountered an incomplete entry");
    }

    let manifest_path = path.join(MANIFEST_FILE);
    let mut manifest_file = open_trusted_cache_file(&manifest_path, MAX_MANIFEST_BYTES)?;
    let manifest_bytes = read_bounded(&mut manifest_file, MAX_MANIFEST_BYTES, "manifest")?;
    let manifest: super::DerivedBackingManifest = serde_json::from_slice(&manifest_bytes)
        .context("derived raster backing manifest is invalid")?;
    super::validate_inventory_manifest(&manifest, key, limits, super::CACHE_SCHEMA_VERSION)?;

    let ready_path = path.join(READY_FILE);
    let mut ready_file = open_trusted_cache_file(&ready_path, MAX_READY_BYTES)?;
    let ready_bytes = read_bounded(&mut ready_file, MAX_READY_BYTES, "ready marker")?;
    let ready =
        std::str::from_utf8(&ready_bytes).context("derived backing ready marker is not UTF-8")?;
    if ready.trim() != sha256_bytes(&manifest_bytes) {
        bail!("derived backing ready marker does not match its manifest");
    }

    let plane = open_trusted_cache_file(&path.join(PLANE_FILE), limits.max_plane_bytes)?;
    if plane.metadata()?.len() != manifest.plane_bytes {
        bail!("derived backing plane length does not match its manifest");
    }
    let lease = open_trusted_cache_file_rw(&path.join(ENTRY_LEASE_FILE), 0)?;
    if lease.metadata()?.len() != 0 {
        bail!("derived backing lease file is not empty");
    }
    let access = open_trusted_cache_file_rw(&path.join(ACCESS_FILE), ACCESS_MARKER_BYTES)?;
    let access_metadata = access.metadata()?;
    if access_metadata.len() != ACCESS_MARKER_BYTES {
        bail!("derived backing access marker has an invalid length");
    }

    let logical_bytes = [
        manifest_file.metadata()?.len(),
        plane.metadata()?.len(),
        ready_file.metadata()?.len(),
        lease.metadata()?.len(),
        access_metadata.len(),
    ]
    .into_iter()
    .try_fold(0_u64, |total, bytes| {
        total
            .checked_add(bytes)
            .context("derived backing entry size overflows")
    })?;
    Ok(CacheEntryInventory {
        key: key.to_owned(),
        path: path.to_owned(),
        logical_bytes,
        last_access: access_metadata.modified()?,
    })
}

fn inventory_legacy_entry(path: &Path, key: &str, limits: DerivedBackingLimits) -> Result<u64> {
    if !is_lower_sha256(key) || !trusted_cache_directory_if_present(path)? {
        bail!("legacy derived backing inventory encountered an invalid entry directory");
    }
    let mut seen = [false; 3];
    for child in fs::read_dir(path)? {
        let child = child?;
        let Some(name) = child.file_name().to_str().map(str::to_owned) else {
            bail!("legacy derived backing entry contains a non-UTF-8 file name");
        };
        let index = match name.as_str() {
            MANIFEST_FILE => 0,
            PLANE_FILE => 1,
            READY_FILE => 2,
            _ => bail!("legacy derived backing entry contains an unexpected file {name}"),
        };
        if std::mem::replace(&mut seen[index], true) {
            bail!("legacy derived backing entry contains a duplicate file {name}");
        }
    }
    if seen.iter().any(|present| !present) {
        bail!("legacy derived backing inventory encountered an incomplete entry");
    }

    let mut manifest_file = open_trusted_cache_file(&path.join(MANIFEST_FILE), MAX_MANIFEST_BYTES)?;
    let manifest_bytes = read_bounded(&mut manifest_file, MAX_MANIFEST_BYTES, "legacy manifest")?;
    let manifest: super::DerivedBackingManifest = serde_json::from_slice(&manifest_bytes)
        .context("legacy derived raster backing manifest is invalid")?;
    super::validate_inventory_manifest(&manifest, key, limits, LEGACY_CACHE_SCHEMA_VERSION)?;

    let mut ready_file = open_trusted_cache_file(&path.join(READY_FILE), MAX_READY_BYTES)?;
    let ready_bytes = read_bounded(&mut ready_file, MAX_READY_BYTES, "legacy ready marker")?;
    let ready = std::str::from_utf8(&ready_bytes)
        .context("legacy derived backing ready marker is not UTF-8")?;
    if ready.trim() != sha256_bytes(&manifest_bytes) {
        bail!("legacy derived backing ready marker does not match its manifest");
    }

    let plane = open_trusted_cache_file(&path.join(PLANE_FILE), limits.max_plane_bytes)?;
    if plane.metadata()?.len() != manifest.plane_bytes {
        bail!("legacy derived backing plane length does not match its manifest");
    }
    [
        manifest_file.metadata()?.len(),
        plane.metadata()?.len(),
        ready_file.metadata()?.len(),
    ]
    .into_iter()
    .try_fold(0_u64, |total, bytes| {
        total
            .checked_add(bytes)
            .context("legacy derived backing entry size overflows")
    })
}

enum EntryExclusiveLease {
    Held(File),
    Contended,
    Missing,
}

fn try_entry_exclusive_lease(entry: &Path) -> Result<EntryExclusiveLease> {
    let path = entry.join(ENTRY_LEASE_FILE);
    match fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(EntryExclusiveLease::Missing);
        }
        Err(error) => return Err(error.into()),
    };
    let file = open_trusted_cache_file_rw(&path, 0)?;
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => {
            if !trusted_cache_directory_if_present(entry)? || !path_refers_to_file(&path, &file)? {
                bail!("derived backing entry changed while its eviction lease was acquired");
            }
            Ok(EntryExclusiveLease::Held(file))
        }
        Err(error) if error.kind() == fs2::lock_contended_error().kind() => {
            Ok(EntryExclusiveLease::Contended)
        }
        Err(error) => Err(error.into()),
    }
}

fn touch_access(path: &Path) -> Result<()> {
    let mut file = open_trusted_cache_file_rw(path, ACCESS_MARKER_BYTES)?;
    if file.metadata()?.len() != ACCESS_MARKER_BYTES {
        bail!("derived backing access marker has an invalid length");
    }
    file.seek(SeekFrom::Start(0))?;
    let marker = b'0' + (ACCESS_COUNTER.fetch_add(1, Ordering::Relaxed) % 10) as u8;
    file.write_all(&[marker])?;
    file.sync_data()?;
    Ok(())
}
