use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use fs2::FileExt;

use super::*;

static STAGED_ASSET_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl DurableProject {
    pub(super) fn stage_asset(&self, reference: &AssetReference, bytes: &[u8]) -> Result<PathBuf> {
        stage_asset_in(&self.materialized_asset_directory(), reference, bytes)
    }

    fn materialized_asset_directory(&self) -> PathBuf {
        std::env::temp_dir()
            .join("spectrum-prism-cache")
            .join(self.info.project_id.to_string())
    }
}

fn stage_asset_in(directory: &Path, reference: &AssetReference, bytes: &[u8]) -> Result<PathBuf> {
    ensure_cache_directory(directory)?;
    let path = directory.join(format!("{}.{}", reference.id, reference.extension));
    if staged_asset_is_valid(&path, reference.id)? {
        return Ok(path);
    }

    let lock_path = directory.join(format!(".{}.lock", reference.id));
    let lock = open_entry_lock(&lock_path)?;
    FileExt::lock_exclusive(&lock)?;
    require_regular_file(&lock_path)?;
    scavenge_stale_temps(directory, reference.id)?;
    if !staged_asset_is_valid(&path, reference.id)? {
        remove_invalid_staged_asset(&path)?;
        let (temporary, mut file) = create_temporary_asset(directory, reference.id)?;
        let publish = (|| -> Result<()> {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &path)?;
            Ok(())
        })();
        if temporary.exists() {
            let _ = fs::remove_file(&temporary);
        }
        publish?;
    }
    if !staged_asset_is_valid(&path, reference.id)? {
        bail!("staged Prism asset {} failed validation", path.display());
    }
    Ok(path)
}

fn open_entry_lock(path: &Path) -> Result<File> {
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            require_regular_file(path)?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(Into::into)
        }
        Err(error) => Err(error.into()),
    }
}

fn create_temporary_asset(directory: &Path, id: AssetId) -> Result<(PathBuf, File)> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..32 {
        let sequence = STAGED_ASSET_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
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
    bail!("could not allocate a unique staged Prism asset temporary file")
}

fn scavenge_stale_temps(directory: &Path, id: AssetId) -> Result<()> {
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

fn ensure_cache_directory(directory: &Path) -> Result<()> {
    let root = directory
        .parent()
        .context("staged Prism cache directory has no parent")?;
    fs::create_dir_all(root)?;
    require_real_directory(root)?;
    match fs::create_dir(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    require_real_directory(directory)
}

fn require_real_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "Prism cache path {} is not a trusted directory",
            path.display()
        );
    }
    Ok(())
}

fn require_regular_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("Prism cache path {} is not a trusted file", path.display());
    }
    Ok(())
}

fn staged_asset_is_valid(path: &Path, expected: AssetId) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(false);
    }
    Ok(AssetId::for_bytes(&fs::read(path)?) == expected)
}

fn remove_invalid_staged_asset(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            bail!("Prism cache asset path {} is a directory", path.display())
        }
        Ok(_) => fs::remove_file(path).map_err(Into::into),
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
    fn concurrent_repair_is_locked_and_scavenges_stale_temporary_files() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("prism-stage-lock-{stamp}"));
        let directory = root.join("cache");
        fs::create_dir_all(&root).unwrap();
        ensure_cache_directory(&directory).unwrap();

        let bytes = Arc::new(b"exact staged bytes".to_vec());
        let id = AssetId::for_bytes(bytes.as_slice());
        let path = directory.join(format!("{id}.png"));
        fs::write(&path, b"corrupt").unwrap();
        let stale = directory.join(format!(".{id}.tmp-stale"));
        fs::write(&stale, b"abandoned").unwrap();

        let workers = 8;
        let barrier = Arc::new(Barrier::new(workers));
        let handles = (0..workers)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let bytes = Arc::clone(&bytes);
                let directory = directory.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let reference = AssetReference {
                        id,
                        extension: "png".into(),
                    };
                    stage_asset_in(&directory, &reference, bytes.as_slice()).unwrap()
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            assert_eq!(handle.join().unwrap(), path);
        }

        assert_eq!(fs::read(&path).unwrap(), bytes.as_slice());
        assert!(!stale.exists());
        let prefix = format!(".{id}.tmp-");
        assert!(!fs::read_dir(&directory).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(&prefix)
        }));
        fs::remove_dir_all(root).unwrap();
    }
}
