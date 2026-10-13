//! The media store: imported footage and audio, copied into the library once
//! and named by their content (`media/<sha256>.<ext>`). Files are too large
//! to embed in revision files, so documents name the file they use and the
//! index records which assets use which file. A file no asset uses is
//! deleted when its last asset is purged.
use crate::{AssetId, Library};
use anyhow::{Context, Result, bail};
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS media_uses(asset TEXT NOT NULL REFERENCES assets(id),
      file TEXT NOT NULL, PRIMARY KEY(asset, file));";

/// A file in the media store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMedia {
    /// Its store name, `<sha256>.<ext>`.
    pub file: String,
    pub bytes: u64,
}

/// How long an unused store file may wait for the asset that imports it.
const UNCLAIMED: Duration = Duration::from_secs(24 * 3600);

impl Library {
    fn media_directory(&self) -> PathBuf {
        self.root.join("media")
    }

    /// Copies `source` into the store, hashing as it copies, and returns its
    /// store name. A file already in the store is not copied twice.
    /// `progress` hears the bytes copied so far.
    pub fn ingest(&self, source: &Path, progress: &mut dyn FnMut(u64)) -> Result<StoredMedia> {
        let directory = self.media_directory();
        fs::create_dir_all(&directory)?;
        let extension = source
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                e.chars()
                    .filter(char::is_ascii_alphanumeric)
                    .take(12)
                    .collect::<String>()
                    .to_ascii_lowercase()
            })
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| "bin".into());
        let mut reader =
            File::open(source).with_context(|| format!("could not read {}", source.display()))?;
        let temporary = directory.join(format!(".incoming-{}", AssetId::new_v4()));
        let copied = (|| -> Result<(String, u64)> {
            let mut writer = File::create(&temporary)?;
            let mut hasher = Sha256::new();
            let mut buffer = vec![0u8; 8 << 20];
            let mut bytes = 0u64;
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                writer.write_all(&buffer[..read])?;
                bytes += read as u64;
                progress(bytes);
            }
            writer.sync_all()?;
            let hash: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            Ok((hash, bytes))
        })();
        let (hash, bytes) = match copied {
            Ok(copied) => copied,
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error.context(format!("could not copy {}", source.display())));
            }
        };
        let file = format!("{hash}.{extension}");
        let destination = directory.join(&file);
        if destination.exists() {
            fs::remove_file(&temporary)?;
        } else {
            let mut permissions = fs::metadata(&temporary)?.permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&temporary, permissions)?;
            fs::rename(&temporary, &destination)?;
        }
        Ok(StoredMedia { file, bytes })
    }

    /// Where a store file is. Only store names address the store.
    pub fn media_path(&self, file: &str) -> Result<PathBuf> {
        if !is_store_name(file) {
            bail!("{file} is not a media store file");
        }
        Ok(self.media_directory().join(file))
    }

    /// Where a rebuildable file made from a store file goes, such as its
    /// playback copy: `media/.cache/<file>.<suffix>`.
    pub fn media_cache_path(&self, file: &str, suffix: &str) -> Result<PathBuf> {
        if !is_store_name(file)
            || !suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.')
        {
            bail!("{file} is not a media store file");
        }
        let directory = self.media_directory().join(".cache");
        fs::create_dir_all(&directory)?;
        Ok(directory.join(format!("{file}.{suffix}")))
    }

    /// Records that `asset` uses a store file.
    pub fn use_media(&self, asset: AssetId, file: &str) -> Result<()> {
        if !is_store_name(file) {
            bail!("{file} is not a media store file");
        }
        self.db.execute(
            "INSERT OR IGNORE INTO media_uses VALUES(?1,?2)",
            params![asset.to_string(), file],
        )?;
        Ok(())
    }

    /// Deletes store files no asset uses: those that `forget` released, and
    /// any left by an import that never finished. Returns the names deleted.
    pub fn delete_unused_media(&self) -> Result<Vec<String>> {
        let Ok(entries) = fs::read_dir(self.media_directory()) else {
            return Ok(Vec::new());
        };
        let mut used = self
            .db
            .prepare("SELECT 1 FROM media_uses WHERE file=?1 LIMIT 1")?;
        let mut deleted = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let stored = is_store_name(&name);
            if !stored && !name.starts_with(".incoming-") {
                continue;
            }
            if stored && used.exists([&name])? {
                continue;
            }
            // A file not claimed yet may belong to an import still running.
            let modified = entry.metadata()?.modified()?;
            let claimed_by_now = !stored
                || SystemTime::now()
                    .duration_since(modified)
                    .is_ok_and(|age| age > UNCLAIMED);
            let released = stored && self.released(&name)?;
            if !(released || claimed_by_now) {
                continue;
            }
            let path = entry.path();
            let mut permissions = fs::metadata(&path)?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(&path, permissions)?;
            fs::remove_file(&path)?;
            // Files made from it go with it.
            if stored && let Ok(made) = fs::read_dir(self.media_directory().join(".cache")) {
                for entry in made.flatten() {
                    if entry.file_name().to_string_lossy().starts_with(&name) {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
            deleted.push(name);
        }
        self.db.execute("DELETE FROM media_released", [])?;
        Ok(deleted)
    }

    fn released(&self, file: &str) -> Result<bool> {
        Ok(self
            .db
            .prepare("SELECT 1 FROM media_released WHERE file=?1 LIMIT 1")?
            .exists([file])?)
    }
}

/// Files whose last use a purge removed, waiting to be deleted.
pub(crate) const RELEASED_SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS media_released(file TEXT PRIMARY KEY);";

/// Whether `name` is a store name: 64 lowercase hex digits, a dot, and a
/// short lowercase alphanumeric extension.
pub fn is_store_name(name: &str) -> bool {
    let Some((hash, extension)) = name.split_once('.') else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && (1..=12).contains(&extension.len())
        && extension
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AssetKind;

    #[test]
    fn media_is_stored_once_and_deleted_with_its_last_asset() {
        let tmp = tempfile::tempdir().unwrap();
        let mut library = Library::open(tmp.path()).unwrap();
        let source = tmp.path().join("Clip 01.MP4");
        fs::write(&source, vec![7u8; 100_000]).unwrap();
        let mut heard = 0;
        let stored = library.ingest(&source, &mut |bytes| heard = bytes).unwrap();
        assert_eq!((stored.bytes, heard), (100_000, 100_000));
        assert!(stored.file.ends_with(".mp4"));
        assert_eq!(library.ingest(&source, &mut |_| {}).unwrap(), stored);
        let path = library.media_path(&stored.file).unwrap();
        assert_eq!(fs::read(&path).unwrap().len(), 100_000);
        assert!(library.media_path("../library.sqlite").is_err());

        let first = library
            .register(
                AssetKind::Footage,
                "one",
                &crate::test_document(tmp.path(), 1),
            )
            .unwrap();
        let second = library
            .register(
                AssetKind::Footage,
                "two",
                &crate::test_document(tmp.path(), 2),
            )
            .unwrap();
        library.use_media(first.id, &stored.file).unwrap();
        library.use_media(second.id, &stored.file).unwrap();
        for asset in [first.id, second.id] {
            library.trash(asset, None).unwrap();
            library.forget(asset).unwrap();
            let deleted = library.delete_unused_media().unwrap();
            assert_eq!(deleted.is_empty(), asset == first.id);
        }
        assert!(!path.exists());
        // An import still under way keeps its file.
        let pending = library.ingest(&source, &mut |_| {}).unwrap();
        assert!(library.delete_unused_media().unwrap().is_empty());
        assert!(library.media_path(&pending.file).unwrap().exists());
        assert!(is_store_name(&pending.file));
        assert!(!is_store_name(&pending.file.to_uppercase()));
    }
}
