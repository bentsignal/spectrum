use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

pub(super) struct TemporaryFont {
    pub(super) path: PathBuf,
}

impl TemporaryFont {
    pub(super) fn new() -> Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        // macOS reports `/var/folders/...`, where `/var` is a symlink. The production
        // font reader correctly rejects symlinked ancestors, so construct this trusted
        // benchmark fixture beneath the canonical temp root instead.
        let temp_root = std::fs::canonicalize(std::env::temp_dir())
            .context("could not canonicalize the benchmark font temp directory")?;
        Self::create_in(&temp_root, stamp)
    }

    fn create_in(temp_root: &Path, stamp: u128) -> Result<Self> {
        for attempt in 0..32 {
            let path = temporary_font_path(temp_root, stamp, attempt);
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = match options.open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("could not create benchmark font fixture"),
            };
            if let Err(error) = file.write_all(epaint_default_fonts::HACK_REGULAR) {
                drop(file);
                let _ = std::fs::remove_file(&path);
                return Err(error).context("could not write benchmark font fixture");
            }
            return Ok(Self { path });
        }
        bail!("could not allocate a unique benchmark font fixture after 32 attempts")
    }
}

fn temporary_font_path(temp_root: &Path, stamp: u128, attempt: u32) -> PathBuf {
    temp_root.join(format!("prism-benchmark-font-{stamp}-{attempt}.ttf"))
}

impl Drop for TemporaryFont {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("prism-benchmark-{label}-{stamp}"));
        std::fs::create_dir(&directory).unwrap();
        directory
    }

    #[test]
    fn uses_the_canonical_temp_root() {
        let font = TemporaryFont::new().unwrap();
        assert_eq!(
            font.path.parent().unwrap(),
            std::fs::canonicalize(std::env::temp_dir()).unwrap()
        );
    }

    #[test]
    fn retries_colliding_leaf_without_overwriting_it() {
        let directory = test_directory("font-collision");
        let collision = temporary_font_path(&directory, 7, 0);
        std::fs::write(&collision, b"existing").unwrap();

        let font = TemporaryFont::create_in(&directory, 7).unwrap();
        assert_eq!(font.path, temporary_font_path(&directory, 7, 1));
        assert_eq!(std::fs::read(&collision).unwrap(), b"existing");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&font.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        drop(font);
        std::fs::remove_file(collision).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_a_symlink_leaf() {
        let directory = test_directory("font-symlink");
        let sentinel = directory.join("sentinel.ttf");
        std::fs::write(&sentinel, b"sentinel").unwrap();
        let symlink = temporary_font_path(&directory, 11, 0);
        std::os::unix::fs::symlink(&sentinel, &symlink).unwrap();

        let font = TemporaryFont::create_in(&directory, 11).unwrap();
        assert_eq!(font.path, temporary_font_path(&directory, 11, 1));
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"sentinel");

        drop(font);
        std::fs::remove_file(symlink).unwrap();
        std::fs::remove_file(sentinel).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
