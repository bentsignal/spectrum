use std::{
    fs,
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use image::{DynamicImage, Rgba, RgbaImage};
use spectrum_imaging::{ExactRegionSource, PixelRegion, SourceSampleDepth};

use super::*;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "prism-sequential-png-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pixels(width: u32, height: u32, seed: u8) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        Rgba([
            seed.wrapping_add((x * 17 + y * 3) as u8),
            seed.wrapping_add((x * 5 + y * 19) as u8),
            seed.wrapping_add((x * 11 + y * 7) as u8),
            80_u8.wrapping_add((x * 13 + y * 23) as u8),
        ])
    })
}

fn expected_region(image: &RgbaImage, region: PixelRegion) -> RgbaImage {
    image::imageops::crop_imm(image, region.x, region.y, region.width, region.height).to_image()
}

#[test]
fn concurrent_reads_use_independent_positioned_cursors() {
    let directory = TestDirectory::new("concurrent");
    let path = directory.path("source.png");
    let image = pixels(257, 193, 31);
    image.save(&path).unwrap();
    let source =
        Arc::new(SequentialPngSource::open(&path, SequentialPngLimits::default()).unwrap());
    let regions = [
        PixelRegion {
            x: 3,
            y: 5,
            width: 61,
            height: 47,
        },
        PixelRegion {
            x: 113,
            y: 71,
            width: 89,
            height: 93,
        },
        PixelRegion {
            x: 17,
            y: 149,
            width: 131,
            height: 31,
        },
    ];
    let expected: Vec<_> = regions
        .iter()
        .copied()
        .map(|region| expected_region(&image, region))
        .collect();
    let workers: Vec<_> = (0..8)
        .map(|worker| {
            let source = Arc::clone(&source);
            let expected = expected.clone();
            std::thread::spawn(move || {
                for iteration in 0..24 {
                    let index = (worker + iteration) % regions.len();
                    assert_eq!(
                        source.read_exact_region(regions[index]).unwrap(),
                        expected[index]
                    );
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
fn retained_source_survives_path_replacement_and_deletion_with_exact_epoch() {
    let directory = TestDirectory::new("lifetime");
    let path = directory.path("source.png");
    let retired = directory.path("retired.png");
    let original = pixels(41, 37, 7);
    let replacement = pixels(41, 37, 211);
    original.save(&path).unwrap();
    let old = SequentialPngSource::open(&path, SequentialPngLimits::default()).unwrap();
    let old_epoch = old.source_epoch().clone();

    fs::rename(&path, &retired).unwrap();
    replacement.save(&path).unwrap();
    let new = SequentialPngSource::open(&path, SequentialPngLimits::default()).unwrap();
    assert_ne!(old_epoch, *new.source_epoch());
    fs::remove_file(&retired).unwrap();

    let region = PixelRegion {
        x: 5,
        y: 9,
        width: 23,
        height: 17,
    };
    assert_eq!(
        old.read_exact_region(region).unwrap(),
        expected_region(&original, region)
    );
    assert_eq!(
        new.read_exact_region(region).unwrap(),
        expected_region(&replacement, region)
    );
}

#[cfg(not(windows))]
#[test]
fn same_length_in_place_mutation_with_restored_mtime_fails_closed() {
    let directory = TestDirectory::new("mutation");
    let path = directory.path("source.png");
    let replacement_path = directory.path("replacement.png");
    let original = pixels(32, 24, 17);
    let replacement = pixels(32, 24, 201);
    write_uncompressed_png(&path, &original);
    write_uncompressed_png(&replacement_path, &replacement);
    let original_bytes = fs::read(&path).unwrap();
    let replacement_bytes = fs::read(&replacement_path).unwrap();
    assert_eq!(original_bytes.len(), replacement_bytes.len());
    assert_ne!(original_bytes, replacement_bytes);

    let source = SequentialPngSource::open(&path, SequentialPngLimits::default()).unwrap();
    let epoch = source.source_epoch().clone();
    let before = source
        .fingerprint
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let metadata = fs::metadata(&path).unwrap();
    let mut writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    writer.seek(SeekFrom::Start(0)).unwrap();
    writer.write_all(&replacement_bytes).unwrap();
    writer.sync_all().unwrap();
    writer
        .set_times(
            fs::FileTimes::new()
                .set_accessed(metadata.accessed().unwrap())
                .set_modified(metadata.modified().unwrap()),
        )
        .unwrap();
    drop(writer);

    let changed_metadata = fs::metadata(&path).unwrap();
    assert_eq!(changed_metadata.len(), metadata.len());
    assert_eq!(
        changed_metadata.modified().unwrap(),
        metadata.modified().unwrap()
    );
    assert_ne!(
        FileFingerprint::from_file(&source.file).unwrap(),
        before,
        "the platform change signal must expose a retained-inode rewrite"
    );

    let error = source
        .read_exact_region(PixelRegion {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        })
        .unwrap_err();
    assert!(matches!(error, SequentialPngReadError::Changed));

    let current = SequentialPngSource::open(&path, SequentialPngLimits::default()).unwrap();
    assert_ne!(current.source_epoch(), &epoch);
    assert_eq!(
        current
            .read_exact_region(PixelRegion {
                x: 0,
                y: 0,
                width: 8,
                height: 8,
            })
            .unwrap(),
        expected_region(
            &replacement,
            PixelRegion {
                x: 0,
                y: 0,
                width: 8,
                height: 8,
            }
        )
    );
}

#[cfg(windows)]
#[test]
fn published_source_denies_a_new_retained_inode_writer() {
    let directory = TestDirectory::new("deny-writer");
    let path = directory.path("source.png");
    let original = pixels(32, 24, 17);
    write_uncompressed_png(&path, &original);
    let source = SequentialPngSource::open(&path, SequentialPngLimits::default()).unwrap();

    let error = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap_err();
    assert_eq!(
        error.raw_os_error(),
        Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
    );
    assert_eq!(
        source
            .read_exact_region(PixelRegion {
                x: 0,
                y: 0,
                width: 8,
                height: 8,
            })
            .unwrap(),
        expected_region(
            &original,
            PixelRegion {
                x: 0,
                y: 0,
                width: 8,
                height: 8,
            }
        )
    );
}

#[test]
fn color_layouts_and_subbyte_depth_expand_to_exact_rgba8() {
    let directory = TestDirectory::new("color-depth");
    let cases = [
        (
            "l8.png",
            DynamicImage::ImageLuma8(image::GrayImage::from_fn(7, 5, |x, y| {
                image::Luma([(x * 31 + y * 17) as u8])
            })),
            "l8",
        ),
        (
            "la8.png",
            DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_fn(7, 5, |x, y| {
                image::LumaA([(x * 31 + y * 17) as u8, (91 + x * 7 + y * 3) as u8])
            })),
            "la8",
        ),
        (
            "rgb8.png",
            DynamicImage::ImageRgb8(image::RgbImage::from_fn(7, 5, |x, y| {
                image::Rgb([x as u8 * 23, y as u8 * 37, (x + y) as u8 * 13])
            })),
            "rgb8",
        ),
        (
            "rgba8.png",
            DynamicImage::ImageRgba8(pixels(7, 5, 53)),
            "rgba8",
        ),
    ];
    for (name, image, encoding) in cases {
        let path = directory.path(name);
        image
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        let expected = image.to_rgba8();
        let source = SequentialPngSource::open(&path, SequentialPngLimits::default()).unwrap();
        assert_eq!(source.info().descriptor.color_encoding, encoding);
        assert_eq!(
            source.info().descriptor.sample_depth,
            SourceSampleDepth::EightBit
        );
        assert_eq!(
            source
                .read_exact_region(PixelRegion {
                    x: 1,
                    y: 1,
                    width: 5,
                    height: 3,
                })
                .unwrap(),
            expected_region(
                &expected,
                PixelRegion {
                    x: 1,
                    y: 1,
                    width: 5,
                    height: 3,
                }
            )
        );
    }

    let one_bit_path = directory.path("one-bit.png");
    write_one_bit_png(&one_bit_path);
    let one_bit = SequentialPngSource::open(&one_bit_path, SequentialPngLimits::default()).unwrap();
    assert_eq!(
        one_bit.info().descriptor.sample_depth,
        SourceSampleDepth::Other(1)
    );
    assert_eq!(
        one_bit
            .read_exact_region(PixelRegion {
                x: 0,
                y: 0,
                width: 8,
                height: 2,
            })
            .unwrap()
            .pixels()
            .map(|pixel| pixel.0)
            .collect::<Vec<_>>(),
        [
            255_u8, 0, 255, 0, 255, 0, 255, 0, 0, 255, 0, 255, 0, 255, 0, 255
        ]
        .into_iter()
        .map(|value| [value, value, value, 255])
        .collect::<Vec<_>>()
    );
}

#[test]
fn sixteen_bit_png_is_rejected_before_publication() {
    let directory = TestDirectory::new("sixteen-bit");
    let path = directory.path("source.png");
    DynamicImage::ImageLuma16(image::ImageBuffer::from_fn(8, 6, |x, y| {
        image::Luma([((x * 997 + y * 313) % 65_536) as u16])
    }))
    .save_with_format(&path, image::ImageFormat::Png)
    .unwrap();
    let error = match SequentialPngSource::open(&path, SequentialPngLimits::default()) {
        Ok(_) => panic!("16-bit PNG unexpectedly produced a sequential provider"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("16-bit PNG"));
}

fn write_one_bit_png(path: &Path) {
    let file = fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, 8, 2);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::One);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[0b1010_1010, 0b0101_0101])
        .unwrap();
}

fn write_uncompressed_png(path: &Path, image: &RgbaImage) {
    let file = fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::NoCompression);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(image.as_raw()).unwrap();
}
