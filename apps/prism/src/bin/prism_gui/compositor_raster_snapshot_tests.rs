use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use image::{DynamicImage, RgbImage, Rgba, RgbaImage};
use prism_core::{
    BlendMode, DerivedBackingCache, DerivedBackingLimits, Document, Layer, LayerKind, LayerMask,
    PixelMask, PrepareDerivedBacking, RasterSourceEpoch, ResolvedRasterSource, SequentialPngLimits,
    SequentialPngSource, Transform,
};

use super::*;
use prism_core::render_document_scaled as render_full_oracle;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "prism-mixed-raster-snapshot-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn source_pixels(width: u32, height: u32, seed: u8) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        Rgba([
            seed.wrapping_add((x * 29 + y * 7) as u8),
            seed.wrapping_add((x * 3 + y * 31) as u8),
            seed.wrapping_add((x * 17 + y * 11) as u8),
            100_u8.wrapping_add((x * 13 + y * 5) as u8),
        ])
    })
}

fn sequential(path: &Path) -> ResolvedRasterSource {
    let source = SequentialPngSource::open(path, SequentialPngLimits::default()).unwrap();
    ResolvedRasterSource::new(source.source_epoch().clone(), Arc::new(source)).unwrap()
}

fn derived(cache: &DerivedBackingCache, path: &Path) -> ResolvedRasterSource {
    let backing = match cache.prepare(path).unwrap() {
        PrepareDerivedBacking::Ready { backing, .. } => backing,
        PrepareDerivedBacking::InProgress(_) => panic!("fresh fixture unexpectedly stayed busy"),
    };
    ResolvedRasterSource::new(
        RasterSourceEpoch::new(backing.key().to_owned()).unwrap(),
        Arc::new(backing),
    )
    .unwrap()
}

fn write_format(path: &Path, format: image::ImageFormat, pixels: &RgbaImage) {
    let image = if format == image::ImageFormat::Jpeg {
        DynamicImage::ImageRgb8(RgbImage::from_fn(
            pixels.width(),
            pixels.height(),
            |x, y| {
                let rgba = pixels.get_pixel(x, y).0;
                image::Rgb([rgba[0], rgba[1], rgba[2]])
            },
        ))
    } else {
        DynamicImage::ImageRgba8(pixels.clone())
    };
    image.save_with_format(path, format).unwrap();
}

fn mixed_document(png: PathBuf, derived: PathBuf, derived_dimensions: (u32, u32)) -> Document {
    let mut document = Document::new("Mixed raster snapshot", 24, 18);
    document.background = [17, 29, 43, 181];
    document.layers.push(Layer {
        id: 1,
        opacity: 0.91,
        transform: Transform {
            scale_x: 2.0,
            scale_y: 2.0,
            ..Transform::default()
        },
        kind: LayerKind::Raster {
            path: png,
            original_path: None,
        },
        ..Layer::default()
    });
    let mask_pixels =
        usize::try_from(u64::from(derived_dimensions.0) * u64::from(derived_dimensions.1)).unwrap();
    document.layers.push(Layer {
        id: 2,
        opacity: 0.73,
        blend_mode: BlendMode::Overlay,
        clip_to_below: true,
        transform: Transform {
            x: 3.0,
            y: 2.0,
            scale_x: 1.35,
            scale_y: 1.15,
            rotation: 13.0,
        },
        mask: LayerMask {
            enabled: true,
            x: 0.08,
            y: 0.12,
            width: 0.8,
            height: 0.74,
            invert: false,
        },
        pixel_mask: Some(PixelMask::new(
            derived_dimensions.0,
            derived_dimensions.1,
            (0..mask_pixels)
                .map(|index| if index % 5 == 0 { 96 } else { 255 })
                .collect::<Vec<_>>(),
        )),
        adjustments: spectrum_imaging::Adjustments {
            exposure: 0.18,
            contrast: 9.0,
            sharpening: 2.0,
            spots: vec![spectrum_imaging::SpotRemoval {
                x: 0.55,
                y: 0.45,
                radius: 0.08,
                opacity: 0.7,
            }],
            ..Default::default()
        },
        kind: LayerKind::Raster {
            path: derived,
            original_path: None,
        },
        ..Layer::default()
    });
    document
}

#[test]
fn png_plus_each_derived_format_stays_exact_at_eight_and_sixteen_x() {
    let directory = TestDirectory::new();
    let png_path = directory.path("sequential.png");
    source_pixels(12, 9, 23).save(&png_path).unwrap();
    let cache = DerivedBackingCache::new(
        directory.path("cache"),
        DerivedBackingLimits {
            max_cache_bytes: 64 * 1_024 * 1_024,
            ..DerivedBackingLimits::default()
        },
    );
    let formats = [
        ("derived.jpg", image::ImageFormat::Jpeg),
        ("derived.webp", image::ImageFormat::WebP),
        ("derived.tiff", image::ImageFormat::Tiff),
    ];
    for (index, (name, format)) in formats.into_iter().enumerate() {
        let derived_path = directory.path(name);
        write_format(
            &derived_path,
            format,
            &source_pixels(12, 9, 71 + index as u8 * 37),
        );
        assert_mixed_scale_parity(&cache, &png_path, &derived_path, (12, 9), 10 + index as u64);
    }

    let adam7_path = directory.path("derived-adam7.png");
    write_one_pixel_adam7(&adam7_path, [137, 61, 211, 189]);
    assert_mixed_scale_parity(&cache, &png_path, &adam7_path, (1, 1), 20);
}

fn assert_mixed_scale_parity(
    cache: &DerivedBackingCache,
    png_path: &Path,
    derived_path: &Path,
    derived_dimensions: (u32, u32),
    epoch: u64,
) {
    let document = mixed_document(
        png_path.to_owned(),
        derived_path.to_owned(),
        derived_dimensions,
    );
    let snapshot = RasterSourceSnapshot::with_test_providers(
        epoch,
        [
            (png_path.to_owned(), sequential(png_path)),
            (derived_path.to_owned(), derived(cache, derived_path)),
        ],
    );
    assert_eq!(
        snapshot.render_mode(&document),
        RasterRenderMode::Provider {
            snapshot_epoch: epoch
        }
    );
    for scale in [8.0, 16.0] {
        let region = prism_core::RenderRegion {
            x: 0,
            y: 0,
            width: (document.width as f32 * scale) as u32,
            height: (document.height as f32 * scale) as u32,
        };
        let oracle = render_full_oracle(&document, scale).unwrap().to_rgba8();
        let (actual, stats) = prism_core::render_document_region_scaled_with_sources_and_stats(
            &document,
            scale,
            region,
            snapshot.as_ref(),
        )
        .unwrap();
        assert_eq!(actual.to_rgba8(), oracle);
        assert_eq!(stats.fallback_decode_bytes, 0);
        assert_eq!(stats.transformed_surface_pixels, 0);
    }
}

#[test]
fn mixed_provider_snapshot_preserves_every_blend_and_mask_direction() {
    let directory = TestDirectory::new();
    let png_path = directory.path("sequential.png");
    let tiff_path = directory.path("derived.tiff");
    source_pixels(12, 9, 19).save(&png_path).unwrap();
    write_format(
        &tiff_path,
        image::ImageFormat::Tiff,
        &source_pixels(12, 9, 101),
    );
    let cache = DerivedBackingCache::new(directory.path("cache"), DerivedBackingLimits::default());
    let snapshot = RasterSourceSnapshot::with_test_providers(
        31,
        [
            (png_path.clone(), sequential(&png_path)),
            (tiff_path.clone(), derived(&cache, &tiff_path)),
        ],
    );
    let mut document = mixed_document(png_path, tiff_path, (12, 9));
    for (index, blend) in BlendMode::ALL.into_iter().enumerate() {
        document.layers[1].blend_mode = blend;
        document.layers[1].mask.invert = index % 2 == 1;
        let scale = 8.0;
        let region = prism_core::RenderRegion {
            x: 0,
            y: 0,
            width: document.width * 8,
            height: document.height * 8,
        };
        let oracle = render_full_oracle(&document, scale).unwrap().to_rgba8();
        let actual = prism_core::render_document_region_scaled_with_sources(
            &document,
            scale,
            region,
            snapshot.as_ref(),
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(actual, oracle, "mixed provider mismatch for {blend:?}");
    }
}

#[test]
fn incomplete_mixed_snapshot_is_capped_and_provider_render_fails_closed() {
    let directory = TestDirectory::new();
    let png_path = directory.path("sequential.png");
    let jpeg_path = directory.path("derived.jpg");
    source_pixels(12, 9, 11).save(&png_path).unwrap();
    write_format(
        &jpeg_path,
        image::ImageFormat::Jpeg,
        &source_pixels(12, 9, 99),
    );
    let document = mixed_document(png_path.clone(), jpeg_path, (12, 9));
    let incomplete =
        RasterSourceSnapshot::with_test_provider(41, png_path.clone(), sequential(&png_path));
    assert_eq!(
        incomplete.render_mode(&document),
        RasterRenderMode::FallbackCapped
    );
    let error = prism_core::render_document_region_scaled_with_sources(
        &document,
        8.0,
        prism_core::RenderRegion {
            x: 0,
            y: 0,
            width: 192,
            height: 144,
        },
        incomplete.as_ref(),
    )
    .unwrap_err();
    let diagnostic = format!("{error:#}");
    assert!(
        diagnostic.contains("cannot use legacy path fallback with a provider resolver"),
        "{diagnostic}"
    );
}

fn write_one_pixel_adam7(path: &Path, rgba: [u8; 4]) {
    let ordinary = RgbaImage::from_pixel(1, 1, Rgba(rgba));
    ordinary.save(path).unwrap();
    let mut bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[12..16], b"IHDR");
    bytes[28] = 1;
    let crc = png_crc32(&bytes[12..29]);
    bytes[29..33].copy_from_slice(&crc.to_be_bytes());
    std::fs::write(path, bytes).unwrap();
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}
