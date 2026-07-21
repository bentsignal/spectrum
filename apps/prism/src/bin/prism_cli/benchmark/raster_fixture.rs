use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use prism_core::{
    DerivedBackingCache, DerivedBackingLimits, PrepareDerivedBacking, RasterSourceEpoch,
    RasterSourceResolver, ResolvedRasterSource,
};

pub(super) struct PreparedRasterFixture {
    _directory: BenchmarkDirectory,
    source_path: PathBuf,
    source: Option<ResolvedRasterSource>,
}

impl PreparedRasterFixture {
    pub(super) fn prepare(width: u32, height: u32) -> Result<(Self, Duration)> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "prism-benchmark-derived-raster-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&root)?;
        let directory = BenchmarkDirectory(root);
        let source_path = directory.0.join("source.tiff");
        let pixels = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([
                (x.wrapping_mul(17) + y.wrapping_mul(3)) as u8,
                (x.wrapping_mul(5) + y.wrapping_mul(19)) as u8,
                (x.wrapping_mul(11) + y.wrapping_mul(7)) as u8,
                255,
            ])
        });
        image::DynamicImage::ImageRgba8(pixels)
            .save_with_format(&source_path, image::ImageFormat::Tiff)?;
        let cache =
            DerivedBackingCache::new(directory.0.join("cache"), DerivedBackingLimits::default());
        let started = Instant::now();
        let backing = match cache.prepare(&source_path)? {
            PrepareDerivedBacking::Ready {
                backing,
                created: true,
                ..
            } => backing,
            PrepareDerivedBacking::Ready { created: false, .. } => {
                bail!("fresh benchmark cache unexpectedly reused a backing")
            }
            PrepareDerivedBacking::InProgress(_) => {
                bail!("fresh benchmark cache unexpectedly reported an active builder")
            }
        };
        let cold_prepare = started.elapsed();
        let epoch = RasterSourceEpoch::new(backing.key().to_owned())?;
        let source = ResolvedRasterSource::new(epoch, Arc::new(backing))?;
        Ok((
            Self {
                _directory: directory,
                source_path,
                source: Some(source),
            },
            cold_prepare,
        ))
    }

    pub(super) fn source_path(&self) -> &Path {
        &self.source_path
    }
}

impl RasterSourceResolver for PreparedRasterFixture {
    fn snapshot_epoch(&self) -> u64 {
        1
    }

    fn resolve(&self, path: &Path) -> Option<ResolvedRasterSource> {
        (path == self.source_path.as_path())
            .then(|| self.source.as_ref().expect("fixture is live").clone())
    }
}

impl Drop for PreparedRasterFixture {
    fn drop(&mut self) {
        self.source.take();
    }
}

struct BenchmarkDirectory(PathBuf);

impl Drop for BenchmarkDirectory {
    fn drop(&mut self) {
        make_tree_writable(&self.0);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn make_tree_writable(path: &Path) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    make_writable(path, &metadata);
    if metadata.is_dir()
        && let Ok(entries) = std::fs::read_dir(path)
    {
        for entry in entries.flatten() {
            make_tree_writable(&entry.path());
        }
    }
}

#[cfg(unix)]
fn make_writable(path: &Path, metadata: &std::fs::Metadata) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o200);
    let _ = std::fs::set_permissions(path, permissions);
}

#[cfg(not(unix))]
fn make_writable(path: &Path, metadata: &std::fs::Metadata) {
    let mut permissions = metadata.permissions();
    permissions.set_readonly(false);
    let _ = std::fs::set_permissions(path, permissions);
}
