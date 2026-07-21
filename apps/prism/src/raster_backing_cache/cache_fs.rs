use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::{Context, Result, bail};

#[cfg(any(unix, windows))]
pub(super) type RetainedPlane = File;

#[cfg(not(any(unix, windows)))]
pub(super) struct RetainedPlane(std::sync::Mutex<File>);

#[cfg(any(unix, windows))]
pub(super) fn retain_plane(file: File) -> RetainedPlane {
    file
}

#[cfg(not(any(unix, windows)))]
pub(super) fn retain_plane(file: File) -> RetainedPlane {
    RetainedPlane(std::sync::Mutex::new(file))
}

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
    if !opened_metadata.is_file() || !path_refers_to_file(path, &file)? {
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
pub(super) fn read_exact_at(
    file: &RetainedPlane,
    buffer: &mut [u8],
    offset: u64,
) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buffer, offset)
}

#[cfg(windows)]
pub(super) fn read_exact_at(
    file: &RetainedPlane,
    mut buffer: &mut [u8],
    mut offset: u64,
) -> io::Result<()> {
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
pub(super) fn read_exact_at(
    plane: &RetainedPlane,
    buffer: &mut [u8],
    offset: u64,
) -> io::Result<()> {
    let mut file = plane
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
pub(super) fn path_refers_to_file(path: &Path, file: &File) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let path_metadata = fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse(&path_metadata) || !path_metadata.is_file() {
        return Ok(false);
    }
    let opened_metadata = file.metadata()?;
    Ok(
        path_metadata.dev() == opened_metadata.dev()
            && path_metadata.ino() == opened_metadata.ino(),
    )
}

#[cfg(windows)]
pub(super) fn path_refers_to_file(path: &Path, file: &File) -> io::Result<bool> {
    let path_metadata = fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse(&path_metadata) || !path_metadata.is_file() {
        return Ok(false);
    }
    let verifier = OpenOptions::new().read(true).open(path)?;
    Ok(windows_file_identity(file)? == windows_file_identity(&verifier)?)
}

#[cfg(windows)]
fn windows_file_identity(file: &File) -> io::Result<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the retained File owns a live handle and `information` points to
    // writable storage of the exact structure required by the Win32 API.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((
        information.dwVolumeSerialNumber,
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    ))
}

#[cfg(not(any(unix, windows)))]
pub(super) fn path_refers_to_file(path: &Path, file: &File) -> io::Result<bool> {
    let path_metadata = fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse(&path_metadata) || !path_metadata.is_file() {
        return Ok(false);
    }
    let opened_metadata = file.metadata()?;
    Ok(path_metadata.len() == opened_metadata.len()
        && path_metadata.modified().ok() == opened_metadata.modified().ok())
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

#[cfg(all(test, not(any(unix, windows))))]
mod fallback_tests {
    use std::{sync::Arc, thread};

    use super::*;

    #[test]
    fn retained_plane_serializes_cursor_based_positioned_reads() {
        let path =
            std::env::temp_dir().join(format!("prism-retained-plane-{}", std::process::id()));
        let first = vec![0x31_u8; 4_096];
        let second = vec![0xc7_u8; 4_096];
        let mut contents = first.clone();
        contents.extend_from_slice(&second);
        fs::write(&path, contents).unwrap();
        let plane = Arc::new(retain_plane(File::open(&path).unwrap()));
        let workers = (0..8)
            .map(|worker| {
                let plane = Arc::clone(&plane);
                let expected = if worker % 2 == 0 {
                    first.clone()
                } else {
                    second.clone()
                };
                thread::spawn(move || {
                    for _ in 0..64 {
                        let mut actual = vec![0; expected.len()];
                        read_exact_at(&plane, &mut actual, (worker % 2 * 4_096) as u64).unwrap();
                        assert_eq!(actual, expected);
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        fs::remove_file(path).unwrap();
    }
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
