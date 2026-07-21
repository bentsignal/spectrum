use std::{
    fs::{self, OpenOptions},
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;

static STAGED_ASSET_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl DurableProject {
    pub(super) fn stage_asset(&self, reference: &AssetReference, bytes: &[u8]) -> Result<PathBuf> {
        let directory = self.materialized_asset_directory();
        ensure_cache_directory(&directory)?;
        let path = directory.join(format!("{}.{}", reference.id, reference.extension));
        if !staged_asset_is_valid(&path, reference.id)? {
            remove_invalid_staged_asset(&path)?;
            let sequence = STAGED_ASSET_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let temporary = directory.join(format!(
                ".{}.tmp-{}-{sequence}",
                reference.id,
                std::process::id()
            ));
            let publish = (|| -> Result<()> {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                file.write_all(bytes)?;
                file.sync_all()?;
                drop(file);
                match fs::rename(&temporary, &path) {
                    Ok(()) => Ok(()),
                    Err(_error) if staged_asset_is_valid(&path, reference.id)? => Ok(()),
                    Err(error) => Err(error.into()),
                }
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

    fn materialized_asset_directory(&self) -> PathBuf {
        std::env::temp_dir()
            .join("spectrum-prism-cache")
            .join(self.info.project_id.to_string())
    }
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
