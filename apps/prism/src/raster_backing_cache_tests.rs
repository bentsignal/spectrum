use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use flate2::{Compression, write::ZlibEncoder};
use fs2::FileExt;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use spectrum_imaging::{ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness};

use crate::{
    DerivedBackingCache, DerivedBackingLimits, PrepareDerivedBacking, inspect_raster_region_source,
};

static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "prism-derived-backing-{label}-{}-{}",
            std::process::id(),
            TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        make_tree_writable(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn make_tree_writable(path: &Path) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    make_writable(path, &metadata);
    if metadata.is_dir()
        && let Ok(entries) = fs::read_dir(path)
    {
        for entry in entries.flatten() {
            make_tree_writable(&entry.path());
        }
    }
}

#[cfg(unix)]
fn make_writable(path: &Path, metadata: &fs::Metadata) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o200);
    let _ = fs::set_permissions(path, permissions);
}

#[cfg(not(unix))]
fn make_writable(path: &Path, metadata: &fs::Metadata) {
    let mut permissions = metadata.permissions();
    permissions.set_readonly(false);
    let _ = fs::set_permissions(path, permissions);
}

fn test_pixels(width: u32, height: u32, seed: u8) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        Rgba([
            (x * 31 + y * 7) as u8 ^ seed,
            (x * 5 + y * 29) as u8 ^ seed.rotate_left(2),
            (x * 17 + y * 11) as u8 ^ seed.rotate_right(1),
            71_u8.wrapping_add((x * 13 + y * 19) as u8),
        ])
    })
}

fn cache(root: &Path) -> DerivedBackingCache {
    DerivedBackingCache::new(root, DerivedBackingLimits::default())
}

fn prepare_ready(cache: &DerivedBackingCache, source: &Path) -> crate::DerivedRasterBacking {
    match cache.prepare(source).unwrap() {
        PrepareDerivedBacking::Ready { backing, .. } => backing,
        PrepareDerivedBacking::InProgress(_) => panic!("test cache unexpectedly busy"),
    }
}

#[test]
fn exact_planes_match_current_jpeg_webp_and_adam7_decoders() {
    let directory = TestDirectory::new("decoder-parity");
    let originals = test_pixels(13, 9, 37);
    let sources = [
        (directory.path().join("small.jpg"), ImageFormat::Jpeg),
        (directory.path().join("small.webp"), ImageFormat::WebP),
    ];
    for (path, format) in &sources {
        DynamicImage::ImageRgba8(originals.clone())
            .save_with_format(path, *format)
            .unwrap();
    }
    let adam7 = directory.path().join("small-adam7.png");
    write_adam7_rgba8(&adam7, &originals);

    for path in sources
        .iter()
        .map(|(path, _)| path)
        .chain(std::iter::once(&adam7))
    {
        let expected = image::open(path).unwrap().to_rgba8();
        let backing = prepare_ready(&cache(&directory.path().join("cache")), path);
        let actual = backing
            .read_exact_region(PixelRegion {
                x: 0,
                y: 0,
                width: expected.width(),
                height: expected.height(),
            })
            .unwrap();
        assert_eq!(actual, expected, "backing mismatch for {}", path.display());
    }
}

#[test]
fn row_seeked_reads_cross_row_boundaries_exactly() {
    let directory = TestDirectory::new("row-boundaries");
    let source = directory.path().join("rows.webp");
    DynamicImage::ImageRgba8(test_pixels(17, 12, 83))
        .save_with_format(&source, ImageFormat::WebP)
        .unwrap();
    let expected = image::open(&source).unwrap().to_rgba8();
    let backing = prepare_ready(&cache(&directory.path().join("cache")), &source);
    let region = PixelRegion {
        x: 13,
        y: 4,
        width: 3,
        height: 5,
    };
    let actual = backing.read_exact_region(region).unwrap();
    let expected =
        image::imageops::crop_imm(&expected, region.x, region.y, region.width, region.height)
            .to_image();
    assert_eq!(actual, expected);
}

#[test]
fn corrupt_or_truncated_plane_is_rejected_before_use() {
    let directory = TestDirectory::new("corrupt");
    let source = directory.path().join("source.jpg");
    DynamicImage::ImageRgba8(test_pixels(10, 8, 17))
        .save_with_format(&source, ImageFormat::Jpeg)
        .unwrap();
    let cache = cache(&directory.path().join("cache"));
    let identity = cache.identify(&source).unwrap();
    let backing = prepare_ready(&cache, &source);
    let plane = backing.plane_path().to_owned();
    make_writable(&plane, &fs::metadata(&plane).unwrap());
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&plane)
        .unwrap();
    assert!(cache.open_ready(&identity).is_err());
}

#[test]
fn source_mutation_changes_the_content_address() {
    let directory = TestDirectory::new("mutation");
    let source = directory.path().join("mutable.webp");
    DynamicImage::ImageRgba8(test_pixels(9, 7, 1))
        .save_with_format(&source, ImageFormat::WebP)
        .unwrap();
    let cache = cache(&directory.path().join("cache"));
    let first = cache.identify(&source).unwrap();
    DynamicImage::ImageRgba8(test_pixels(9, 7, 211))
        .save_with_format(&source, ImageFormat::WebP)
        .unwrap();
    let second = cache.identify(&source).unwrap();
    assert_ne!(first.source_sha256(), second.source_sha256());
    assert_ne!(first.key(), second.key());
}

#[test]
fn derived_capability_is_not_ready_before_atomic_publication() {
    let directory = TestDirectory::new("not-ready");
    let source = directory.path().join("source.jpg");
    DynamicImage::ImageRgba8(test_pixels(8, 6, 53))
        .save_with_format(&source, ImageFormat::Jpeg)
        .unwrap();
    let inspected = inspect_raster_region_source(&source).unwrap();
    assert_eq!(
        inspected.info.capability,
        RegionReadCapability::DerivedBacking
    );
    assert_eq!(inspected.info.readiness, RegionReadiness::NeedsPreparation);
    let cache = cache(&directory.path().join("cache"));
    let identity = cache.identify(&source).unwrap();
    assert!(cache.open_ready(&identity).unwrap().is_none());
}

#[test]
fn prepare_lock_is_nonblocking_and_released_for_the_next_builder() {
    let directory = TestDirectory::new("single-flight");
    let source = directory.path().join("source.webp");
    DynamicImage::ImageRgba8(test_pixels(8, 6, 91))
        .save_with_format(&source, ImageFormat::WebP)
        .unwrap();
    let cache_root = directory.path().join("cache");
    fs::create_dir(&cache_root).unwrap();
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(cache_root.join(".prepare-lock"))
        .unwrap();
    lock.try_lock_exclusive().unwrap();
    match cache(&cache_root).prepare(&source).unwrap() {
        PrepareDerivedBacking::InProgress(identity) => {
            assert_eq!(identity.descriptor().width, 8);
        }
        PrepareDerivedBacking::Ready { .. } => panic!("locked cache unexpectedly prepared"),
    }
    FileExt::unlock(&lock).unwrap();
    assert!(matches!(
        cache(&cache_root).prepare(&source).unwrap(),
        PrepareDerivedBacking::Ready { created: true, .. }
    ));
}

#[test]
fn noninterlaced_png_stays_sequential_and_adam7_requires_backing() {
    let directory = TestDirectory::new("png-policy");
    let pixels = test_pixels(9, 7, 31);
    let sequential = directory.path().join("sequential.png");
    pixels.save(&sequential).unwrap();
    let adam7 = directory.path().join("adam7.png");
    write_adam7_rgba8(&adam7, &pixels);
    let sequential = inspect_raster_region_source(&sequential).unwrap().info;
    let adam7 = inspect_raster_region_source(&adam7).unwrap().info;
    assert_eq!(
        sequential.capability,
        RegionReadCapability::SequentialBounded
    );
    assert_eq!(sequential.readiness, RegionReadiness::Ready);
    assert_eq!(adam7.capability, RegionReadCapability::DerivedBacking);
    assert_eq!(adam7.readiness, RegionReadiness::NeedsPreparation);
}

#[test]
fn sixteen_bit_sources_fall_back_and_tiff_is_only_a_future_capability() {
    let directory = TestDirectory::new("precision-policy");
    let sixteen_bit = directory.path().join("sixteen.png");
    DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(
        6,
        4,
        image::Rgba([123_u16, 4_567, 32_000, u16::MAX]),
    ))
    .save(&sixteen_bit)
    .unwrap();
    let tiff = directory.path().join("tiles-later.tiff");
    DynamicImage::ImageRgba8(test_pixels(6, 4, 19))
        .save_with_format(&tiff, ImageFormat::Tiff)
        .unwrap();

    let sixteen_bit = inspect_raster_region_source(&sixteen_bit).unwrap().info;
    assert_eq!(sixteen_bit.capability, RegionReadCapability::FullDecodeOnly);
    assert_eq!(sixteen_bit.readiness, RegionReadiness::Unsupported);
    let tiff = inspect_raster_region_source(&tiff).unwrap().info;
    assert_eq!(tiff.capability, RegionReadCapability::SeekableChunks);
    assert_eq!(tiff.readiness, RegionReadiness::Unsupported);
}

#[test]
fn plane_and_region_limits_are_enforced_before_large_work() {
    let directory = TestDirectory::new("limits");
    let source = directory.path().join("source.webp");
    DynamicImage::ImageRgba8(test_pixels(8, 6, 101))
        .save_with_format(&source, ImageFormat::WebP)
        .unwrap();
    let tiny_plane = DerivedBackingCache::new(
        directory.path().join("tiny-plane-cache"),
        DerivedBackingLimits {
            max_plane_bytes: 64,
            ..DerivedBackingLimits::default()
        },
    );
    assert!(tiny_plane.prepare(&source).is_err());

    let tiny_regions = DerivedBackingCache::new(
        directory.path().join("tiny-region-cache"),
        DerivedBackingLimits {
            max_region_pixels: 4,
            ..DerivedBackingLimits::default()
        },
    );
    let backing = prepare_ready(&tiny_regions, &source);
    assert!(
        backing
            .read_exact_region(PixelRegion {
                x: 0,
                y: 0,
                width: 3,
                height: 2,
            })
            .is_err()
    );
}

fn write_adam7_rgba8(path: &Path, pixels: &RgbaImage) {
    let mut filtered = Vec::new();
    for &(start_x, start_y, step_x, step_y) in &[
        (0, 0, 8, 8),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ] {
        for y in (start_y..pixels.height()).step_by(step_y) {
            if start_x >= pixels.width() {
                continue;
            }
            filtered.push(0);
            for x in (start_x..pixels.width()).step_by(step_x) {
                filtered.extend_from_slice(&pixels.get_pixel(x, y).0);
            }
        }
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&filtered).unwrap();
    let compressed = encoder.finish().unwrap();
    let mut png = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
    let mut header = Vec::new();
    header.extend_from_slice(&pixels.width().to_be_bytes());
    header.extend_from_slice(&pixels.height().to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 1]);
    write_png_chunk(&mut png, b"IHDR", &header);
    write_png_chunk(&mut png, b"IDAT", &compressed);
    write_png_chunk(&mut png, b"IEND", &[]);
    fs::write(path, png).unwrap();
}

fn write_png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut checksum_input = Vec::with_capacity(kind.len() + data.len());
    checksum_input.extend_from_slice(kind);
    checksum_input.extend_from_slice(data);
    output.extend_from_slice(&crc32(&checksum_input).to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}
