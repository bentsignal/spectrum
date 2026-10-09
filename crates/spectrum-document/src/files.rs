//! Files a document uses (images, fonts, captured sources) are embedded in
//! its revision file by content. Stored documents and commands refer to
//! them as `spectrum-asset:<sha256>.<ext>`; in memory they refer to a copy
//! staged in the document's cache, which is rebuilt from the file on demand.
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use spectrum_revisions::{Asset, AssetId};

const PREFIX: &str = "spectrum-asset:";
/// The largest file a document embeds.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// What a file is to its document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileRole {
    /// Content the document shows, such as an image or a font.
    Content,
    /// Content captured earlier, whose SHA-256 the document recorded; the
    /// file must still match it.
    Identified(String),
}

/// Whether a path is a stored reference to an embedded file rather than a
/// file on disk.
pub fn is_embedded_reference(path: &Path) -> bool {
    Reference::parse(path).is_some()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reference {
    pub(crate) id: AssetId,
    pub(crate) extension: String,
}

impl Reference {
    pub(crate) fn parse(path: &Path) -> Option<Self> {
        let value = path.to_str()?.strip_prefix(PREFIX)?;
        let (hash, extension) = value.split_once('.')?;
        let id = AssetId::from_hex(hash)?;
        let extension = sanitize_extension(extension);
        (!extension.is_empty()).then_some(Self { id, extension })
    }

    pub(crate) fn path(&self) -> PathBuf {
        PathBuf::from(format!("{PREFIX}{}.{}", self.id, self.extension))
    }
}

/// Reads a file to embed, checking it against its role.
pub(crate) fn embed(path: &Path, role: &FileRole) -> Result<(Reference, Asset)> {
    if Reference::parse(path).is_some() {
        bail!("{} is already an embedded file", path.display());
    }
    let metadata = fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a file", path.display());
    }
    if metadata.len() > MAX_FILE_BYTES {
        bail!(
            "{} is larger than the {} MiB a document embeds",
            path.display(),
            MAX_FILE_BYTES >> 20
        );
    }
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    hashed(bytes.len());
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(sanitize_extension)
        .filter(|extension| !extension.is_empty())
        .unwrap_or_else(|| "bin".into());
    let asset = Asset::new(media_type(&extension), bytes);
    if let FileRole::Identified(expected) = role
        && asset.id.to_string() != *expected
    {
        bail!("{} no longer matches its recorded content", path.display());
    }
    Ok((
        Reference {
            id: asset.id,
            extension,
        },
        asset,
    ))
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
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static HASHED_BYTES: AtomicU64 = AtomicU64::new(0);

/// Bytes of embedded files this process has read and hashed, to embed them
/// or to check a staged copy. Interaction tests bound it: re-hashing a large
/// photo on every edit is slow everywhere.
pub fn hashed_bytes() -> u64 {
    HASHED_BYTES.load(Ordering::Relaxed)
}

fn hashed(bytes: usize) {
    HASHED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}
static PROCESS_LOCKS: OnceLock<[Mutex<()>; 64]> = OnceLock::new();

/// Writes an embedded file into `directory` (once; later calls reuse a copy
/// that still matches), returning its path. Safe across threads and
/// processes staging the same file.
pub(crate) fn stage(directory: &Path, reference: &Reference, bytes: &[u8]) -> Result<PathBuf> {
    ensure_directory(directory)?;
    let path = directory.join(format!("{}.{}", reference.id, reference.extension));
    if staged_is_valid(&path, reference.id)? {
        return Ok(path);
    }
    // `flock` is process-scoped on macOS, so threads in one process could
    // otherwise enter together and remove each other's temporary files.
    let _process = lock_in_process(reference.id);
    let lock_path = directory.join(format!(".{}.lock", reference.id));
    let lock = open_lock(&lock_path)?;
    FileExt::lock_exclusive(&lock)?;
    require_regular_file(&lock_path)?;
    scavenge_temporaries(directory, reference.id)?;
    if !staged_is_valid(&path, reference.id)? {
        remove_invalid(&path)?;
        let (temporary, mut file) = create_temporary(directory, reference.id)?;
        let written = (|| -> Result<()> {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &path)?;
            Ok(())
        })();
        if temporary.exists() {
            let _ = fs::remove_file(&temporary);
        }
        written?;
    }
    if !staged_is_valid(&path, reference.id)? {
        bail!("staged file {} failed validation", path.display());
    }
    Ok(path)
}

fn lock_in_process(id: AssetId) -> MutexGuard<'static, ()> {
    let locks = PROCESS_LOCKS.get_or_init(|| std::array::from_fn(|_| Mutex::new(())));
    locks[usize::from(id.as_bytes()[0]) % locks.len()]
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn open_lock(path: &Path) -> Result<File> {
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            require_regular_file(path)?;
            Ok(OpenOptions::new().read(true).write(true).open(path)?)
        }
        Err(error) => Err(error.into()),
    }
}

fn create_temporary(directory: &Path, id: AssetId) -> Result<(PathBuf, File)> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..32 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            ".{id}.tmp-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!("could not allocate a unique temporary staged file")
}

fn scavenge_temporaries(directory: &Path, id: AssetId) -> Result<()> {
    let prefix = format!(".{id}.tmp-");
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with(&prefix) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_file() || metadata.file_type().is_symlink() {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn ensure_directory(directory: &Path) -> Result<()> {
    fs::create_dir_all(directory)?;
    let metadata = fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "cache path {} is not a trusted directory",
            directory.display()
        );
    }
    Ok(())
}

fn require_regular_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("cache path {} is not a trusted file", path.display());
    }
    Ok(())
}

/// Staged copies already checked by this process, by path, with the file
/// identity they had then. A copy that has not changed is not read again,
/// so opening a document does not re-hash its large photos every time.
type CheckedCopies = std::collections::HashMap<PathBuf, (FileIdentity, AssetId)>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    length: u64,
    modified: Option<SystemTime>,
    inode: u64,
}

impl FileIdentity {
    fn of(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        let inode = std::os::unix::fs::MetadataExt::ino(metadata);
        #[cfg(not(unix))]
        let inode = 0;
        Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
            inode,
        }
    }
}

fn checked_copies() -> MutexGuard<'static, CheckedCopies> {
    static CHECKED: OnceLock<Mutex<CheckedCopies>> = OnceLock::new();
    CHECKED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn staged_is_valid(path: &Path, expected: AssetId) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(false);
    }
    let identity = FileIdentity::of(&metadata);
    if checked_copies().get(path) == Some(&(identity, expected)) {
        return Ok(true);
    }
    match fs::read(path) {
        Ok(bytes) => {
            hashed(bytes.len());
            let valid = AssetId::for_bytes(&bytes) == expected;
            if valid {
                checked_copies().insert(path.to_path_buf(), (identity, expected));
            }
            Ok(valid)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn remove_invalid(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            bail!("cache file path {} is a directory", path.display())
        }
        Ok(_) => Ok(fs::remove_file(path)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
    };

    use super::*;

    #[test]
    fn concurrent_staging_is_locked_and_repairs_corrupt_copies() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("cache");
        ensure_directory(&directory).unwrap();
        let bytes = Arc::new(b"exact staged bytes".to_vec());
        let id = AssetId::for_bytes(bytes.as_slice());
        let path = directory.join(format!("{id}.png"));
        fs::write(&path, b"corrupt").unwrap();
        let stale = directory.join(format!(".{id}.tmp-stale"));
        fs::write(&stale, b"abandoned").unwrap();
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (barrier, bytes, directory) =
                    (barrier.clone(), bytes.clone(), directory.clone());
                thread::spawn(move || {
                    barrier.wait();
                    let reference = Reference {
                        id,
                        extension: "png".into(),
                    };
                    stage(&directory, &reference, bytes.as_slice()).unwrap()
                })
            })
            .collect();
        for handle in handles {
            assert_eq!(handle.join().unwrap(), path);
        }
        assert_eq!(fs::read(&path).unwrap(), bytes.as_slice());
        assert!(!stale.exists());
    }

    #[test]
    fn references_round_trip_and_embedding_checks_identity() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("photo.JPG");
        fs::write(&file, b"pixels").unwrap();
        let (reference, asset) = embed(&file, &FileRole::Content).unwrap();
        assert_eq!(reference.extension, "jpg");
        assert_eq!(Reference::parse(&reference.path()), Some(reference.clone()));
        assert!(embed(&file, &FileRole::Identified(asset.id.to_string())).is_ok());
        assert!(embed(&file, &FileRole::Identified("0".repeat(64))).is_err());
        assert!(embed(&reference.path(), &FileRole::Content).is_err());
    }
}
