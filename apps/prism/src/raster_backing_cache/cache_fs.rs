use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::{Context, Result, bail};

pub(super) fn trusted_cache_directory_if_present(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
                bail!("cache path {} is not a trusted directory", path.display());
            }
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn trusted_cache_directory(path: &Path) -> Result<()> {
    if trusted_cache_directory_if_present(path)? {
        Ok(())
    } else {
        bail!("cache directory {} is missing", path.display())
    }
}

pub(super) fn trusted_cache_file_length(path: &Path, max_bytes: u64) -> Result<Option<u64>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() || metadata.len() > max_bytes {
        return Ok(None);
    }
    Ok(Some(metadata.len()))
}

pub(super) fn open_trusted_cache_file(path: &Path, max_bytes: u64) -> Result<File> {
    let path_metadata = fs::symlink_metadata(path)
        .with_context(|| format!("could not inspect cache file {}", path.display()))?;
    if metadata_is_link_or_reparse(&path_metadata) || !path_metadata.is_file() {
        bail!(
            "cache file {} is not a trusted regular file",
            path.display()
        );
    }
    if path_metadata.len() > max_bytes {
        bail!("cache file {} exceeds its byte limit", path.display());
    }
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("could not open cache file {}", path.display()))?;
    let opened_metadata = file.metadata()?;
    if !opened_metadata.is_file() || !same_file_identity(&path_metadata, &opened_metadata) {
        bail!("cache file {} changed while it was opened", path.display());
    }
    if opened_metadata.len() > max_bytes {
        bail!("cache file {} exceeds its byte limit", path.display());
    }
    Ok(file)
}

pub(super) fn read_bounded(file: &mut File, max_bytes: u64, label: &str) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        bail!("derived raster backing {label} exceeds its byte limit");
    }
    Ok(bytes)
}

pub(super) fn is_cache_key(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn is_temporary_entry_name(value: &str) -> bool {
    let Some(value) = value.strip_prefix(".tmp-") else {
        return false;
    };
    let mut fields = value.split('-');
    let (Some(key), Some(process), Some(counter), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    is_cache_key(key)
        && !process.is_empty()
        && process.bytes().all(|byte| byte.is_ascii_digit())
        && !counter.is_empty()
        && counter.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(unix)]
pub(super) fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buffer, offset)
}

#[cfg(windows)]
pub(super) fn read_exact_at(file: &File, mut buffer: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;

    while !buffer.is_empty() {
        let read = file.seek_read(buffer, offset)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "derived backing plane ended during positioned read",
            ));
        }
        buffer = &mut buffer[read..];
        offset = offset
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("positioned read offset overflows"))?;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub(super) fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)
}

pub(super) fn remove_cache_entry(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse(&metadata) {
        return unlink_cache_link(path, &metadata);
    }
    if metadata.is_file() {
        make_cache_path_writable(path, &metadata)?;
        return fs::remove_file(path);
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cache entry has an unsupported file type",
        ));
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !(file_type.is_file() || file_type.is_dir() || file_type.is_symlink()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cache entry contains an unsupported file type",
            ));
        }
        remove_cache_entry(&entry.path())?;
    }
    make_cache_path_writable(path, &metadata)?;
    fs::remove_dir(path)
}

#[cfg(unix)]
pub(super) fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
pub(super) fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(any(unix, windows)))]
pub(super) fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(unix)]
pub(super) fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
pub(super) fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    left.volume_serial_number().is_some()
        && left.volume_serial_number() == right.volume_serial_number()
        && left.file_index().is_some()
        && left.file_index() == right.file_index()
}

#[cfg(not(any(unix, windows)))]
pub(super) fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len() && left.modified().ok() == right.modified().ok()
}

#[cfg(unix)]
fn unlink_cache_link(path: &Path, _metadata: &fs::Metadata) -> io::Result<()> {
    fs::remove_file(path)
}

#[cfg(windows)]
fn unlink_cache_link(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.is_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(not(any(unix, windows)))]
fn unlink_cache_link(path: &Path, _metadata: &fs::Metadata) -> io::Result<()> {
    fs::remove_file(path)
}

#[cfg(not(windows))]
fn make_cache_path_writable(_path: &Path, _metadata: &fs::Metadata) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn make_cache_path_writable(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    let mut permissions = metadata.permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)
}

#[cfg(unix)]
pub(super) fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}
