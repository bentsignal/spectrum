use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use prism_core::{
    DerivedBackingCache, DerivedBackingLimits, DerivedBackingReadError, DerivedRasterBacking,
    PrepareDerivedBacking, RasterSourceEpoch, RasterSourceResolver, ResolvedRasterSource,
};
use spectrum_imaging::{ExactRegionSource, PixelRegion, RegionSourceInfo};

pub(super) struct PreparedRasterFixture {
    _directory: BenchmarkDirectory,
    source_path: PathBuf,
    source: Option<ResolvedRasterSource>,
    max_region_pixels: Arc<AtomicU64>,
}

impl PreparedRasterFixture {
    pub(super) fn prepare(width: u32, height: u32) -> Result<(Self, Duration)> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let temporary_root =
            std::fs::canonicalize(std::env::temp_dir()).unwrap_or_else(|_| std::env::temp_dir());
        let root = temporary_root.join(format!(
            "prism-benchmark-derived-raster-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&root)?;
        let directory = BenchmarkDirectory(root);
        let source_path = directory.0.join("source.tiff");
        write_nonuniform_tiff(&source_path, width, height)?;
        let cache =
            DerivedBackingCache::new(directory.0.join("cache"), DerivedBackingLimits::default());
        let identity = cache.identify(&source_path)?;
        let started = Instant::now();
        let backing = match cache.prepare_identified(&source_path, &identity)? {
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
        let max_region_pixels = Arc::new(AtomicU64::new(0));
        let source = ResolvedRasterSource::new_authenticated(
            epoch,
            identity.source_sha256().to_owned(),
            Arc::new(TrackedBacking {
                backing,
                max_region_pixels: Arc::clone(&max_region_pixels),
            }),
        )?;
        Ok((
            Self {
                _directory: directory,
                source_path,
                source: Some(source),
                max_region_pixels,
            },
            cold_prepare,
        ))
    }

    pub(super) fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub(super) fn max_region_pixels(&self) -> u64 {
        self.max_region_pixels.load(Ordering::Relaxed)
    }
}

fn write_nonuniform_tiff(path: &Path, width: u32, height: u32) -> Result<()> {
    let file = File::create(path)?;
    let mut encoder = tiff::encoder::TiffEncoder::new(BufWriter::new(file))?;
    let mut image = encoder.new_image::<tiff::encoder::colortype::Gray8>(width, height)?;
    image.rows_per_strip(8)?;
    let mut row = vec![0u8; width as usize * 8];
    let mut y = 0u32;
    while image.next_strip_sample_count() > 0 {
        let samples = image.next_strip_sample_count() as usize;
        for (index, pixel) in row[..samples].iter_mut().enumerate() {
            let local_y = index / width as usize;
            let x = index % width as usize;
            *pixel = (x as u32)
                .wrapping_mul(17)
                .wrapping_add((y + local_y as u32).wrapping_mul(29)) as u8;
        }
        image.write_strip(&row[..samples])?;
        y += (samples / width as usize) as u32;
    }
    image.finish()?;
    Ok(())
}

struct TrackedBacking {
    backing: DerivedRasterBacking,
    max_region_pixels: Arc<AtomicU64>,
}

impl ExactRegionSource for TrackedBacking {
    type Error = DerivedBackingReadError;

    fn info(&self) -> &RegionSourceInfo {
        self.backing.info()
    }

    fn read_exact_region(&self, region: PixelRegion) -> Result<image::RgbaImage, Self::Error> {
        self.max_region_pixels.fetch_max(
            u64::from(region.width) * u64::from(region.height),
            Ordering::Relaxed,
        );
        self.backing.read_exact_region(region)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_fixture_is_a_nonuniform_authenticated_derived_backing() {
        let (fixture, _) = PreparedRasterFixture::prepare(64, 48).unwrap();
        let source = fixture.resolve(fixture.source_path()).unwrap();
        assert!(source.content_sha256().is_some());
        let pixels = source
            .source()
            .read_exact_region(PixelRegion {
                x: 7,
                y: 9,
                width: 8,
                height: 6,
            })
            .unwrap();
        assert_ne!(pixels.get_pixel(0, 0), pixels.get_pixel(1, 0));
        assert_ne!(pixels.get_pixel(0, 0), pixels.get_pixel(0, 1));
        assert_eq!(fixture.max_region_pixels(), 48);
    }
}

#[cfg(not(unix))]
fn make_writable(path: &Path, metadata: &std::fs::Metadata) {
    let mut permissions = metadata.permissions();
    permissions.set_readonly(false);
    let _ = std::fs::set_permissions(path, permissions);
}
