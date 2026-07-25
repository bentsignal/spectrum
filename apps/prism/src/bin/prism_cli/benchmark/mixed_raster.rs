use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, bail};
use image::{DynamicImage, RgbImage, Rgba, RgbaImage};
use prism_core::{
    BlendMode, DerivedBackingCache, DerivedBackingLimits, Document, Layer, LayerKind, LayerMask,
    PixelMask, PrepareDerivedBacking, RasterSourceEpoch, RasterSourceResolver, RenderRegion,
    ResolvedRasterSource, SequentialPngLimits, SequentialPngSource, Transform,
    render_document_region_scaled_with_sources_and_stats,
};

const DOCUMENT_SIZE: u32 = 16_384;
const VIEWPORT_WIDTH: u32 = 960;
const VIEWPORT_HEIGHT: u32 = 540;
const SAMPLE_COUNT: usize = 5;
const VIEW_DOCUMENT_X: u32 = 11_200;
const VIEW_DOCUMENT_Y: u32 = 8_700;

pub(super) struct MixedRasterMeasurement {
    pub samples_8x: Vec<f64>,
    pub samples_16x: Vec<f64>,
    pub cold_prepare: Duration,
    pub max_source_staging_pixels: u64,
    pub full_plane_copy_bytes: u64,
}

pub(super) fn measure() -> Result<MixedRasterMeasurement> {
    let fixture = MixedRasterFixture::prepare()?;
    let mut samples_8x = Vec::with_capacity(SAMPLE_COUNT);
    let mut samples_16x = Vec::with_capacity(SAMPLE_COUNT);
    let mut max_source_staging_pixels = 0_u64;
    for (scale, samples) in [(8.0, &mut samples_8x), (16.0, &mut samples_16x)] {
        let region = RenderRegion {
            x: (VIEW_DOCUMENT_X as f32 * scale) as u32,
            y: (VIEW_DOCUMENT_Y as f32 * scale) as u32,
            width: VIEWPORT_WIDTH,
            height: VIEWPORT_HEIGHT,
        };
        for _ in 0..SAMPLE_COUNT {
            let started = Instant::now();
            let (rendered, stats) = render_document_region_scaled_with_sources_and_stats(
                &fixture.document,
                scale,
                region,
                &fixture.resolver,
            )?;
            samples.push(started.elapsed().as_secs_f64() * 1_000.0);
            if (rendered.width(), rendered.height()) != (VIEWPORT_WIDTH, VIEWPORT_HEIGHT) {
                bail!("mixed raster benchmark returned incorrect viewport dimensions");
            }
            if stats.fallback_decode_bytes != 0
                || stats.transformed_surface_pixels != 0
                || stats.max_source_staging_pixels >= fixture.minimum_full_source_pixels
                || stats.max_source_staging_pixels > 4_096 * 4_096
                || stats.max_adjusted_staging_pixels > 4_096 * 4_096
            {
                bail!("mixed raster benchmark violated its bounded provider contract");
            }
            max_source_staging_pixels =
                max_source_staging_pixels.max(stats.max_source_staging_pixels);
        }
    }
    Ok(MixedRasterMeasurement {
        samples_8x,
        samples_16x,
        cold_prepare: fixture.cold_prepare,
        max_source_staging_pixels,
        full_plane_copy_bytes: fixture.full_plane_copy_bytes,
    })
}

struct MixedRasterFixture {
    resolver: FixtureResolver,
    document: Document,
    _directory: BenchmarkDirectory,
    cold_prepare: Duration,
    full_plane_copy_bytes: u64,
    minimum_full_source_pixels: u64,
}

impl MixedRasterFixture {
    fn prepare() -> Result<Self> {
        let directory = BenchmarkDirectory::new()?;
        let png_path = directory.path("sequential.png");
        let jpeg_path = directory.path("derived.jpg");
        let tiff_path = directory.path("derived.tiff");
        let png_pixels = RgbaImage::from_fn(DOCUMENT_SIZE, 256, |x, y| {
            Rgba([
                (x.wrapping_mul(17) + y.wrapping_mul(3)) as u8,
                (x.wrapping_mul(5) + y.wrapping_mul(19)) as u8,
                (x.wrapping_mul(11) + y.wrapping_mul(7)) as u8,
                255,
            ])
        });
        png_pixels.save(&png_path)?;
        let derived_pixels = RgbaImage::from_fn(2_048, 1_024, |x, y| {
            Rgba([
                (x.wrapping_mul(13) + y.wrapping_mul(23)) as u8,
                (x.wrapping_mul(29) + y.wrapping_mul(7)) as u8,
                (x.wrapping_mul(3) + y.wrapping_mul(31)) as u8,
                255,
            ])
        });
        let jpeg_pixels =
            RgbImage::from_fn(derived_pixels.width(), derived_pixels.height(), |x, y| {
                let pixel = derived_pixels.get_pixel(x, y).0;
                image::Rgb([pixel[0], pixel[1], pixel[2]])
            });
        DynamicImage::ImageRgb8(jpeg_pixels)
            .save_with_format(&jpeg_path, image::ImageFormat::Jpeg)?;
        DynamicImage::ImageRgba8(derived_pixels.clone())
            .save_with_format(&tiff_path, image::ImageFormat::Tiff)?;

        let started = Instant::now();
        let sequential = SequentialPngSource::open(&png_path, SequentialPngLimits::default())?;
        let sequential =
            ResolvedRasterSource::new(sequential.source_epoch().clone(), Arc::new(sequential))?;
        let cache =
            DerivedBackingCache::new(directory.path("cache"), DerivedBackingLimits::default());
        let (jpeg, jpeg_copy_bytes) = prepare_derived(&cache, &jpeg_path)?;
        let (tiff, tiff_copy_bytes) = prepare_derived(&cache, &tiff_path)?;
        let cold_prepare = started.elapsed();
        let resolver = FixtureResolver {
            sources: HashMap::from([
                (png_path.clone(), sequential),
                (jpeg_path.clone(), jpeg),
                (tiff_path.clone(), tiff),
            ]),
        };
        let document = mixed_document(png_path, jpeg_path, tiff_path);
        Ok(Self {
            resolver,
            document,
            _directory: directory,
            cold_prepare,
            full_plane_copy_bytes: jpeg_copy_bytes.saturating_add(tiff_copy_bytes),
            minimum_full_source_pixels: u64::from(2_048_u32 * 1_024),
        })
    }
}

fn prepare_derived(
    cache: &DerivedBackingCache,
    path: &Path,
) -> Result<(ResolvedRasterSource, u64)> {
    let (backing, memory_plan) = match cache.prepare(path)? {
        PrepareDerivedBacking::Ready {
            backing,
            memory_plan,
            ..
        } => (backing, memory_plan),
        PrepareDerivedBacking::InProgress(_) => {
            bail!("fresh mixed raster benchmark cache unexpectedly stayed busy")
        }
    };
    let source = ResolvedRasterSource::new(
        RasterSourceEpoch::new(backing.key().to_owned())?,
        Arc::new(backing),
    )?;
    Ok((source, memory_plan.full_plane_copy_bytes()))
}

fn mixed_document(png: PathBuf, jpeg: PathBuf, tiff: PathBuf) -> Document {
    let mut document = Document::new("16K mixed raster viewport", DOCUMENT_SIZE, DOCUMENT_SIZE);
    document.background = [19, 29, 43, 255];
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            y: 8_600.0,
            ..Transform::default()
        },
        kind: LayerKind::Raster {
            path: png,
            original_path: None,
        },
        ..Layer::default()
    });
    document.layers.push(Layer {
        id: 2,
        opacity: 0.81,
        blend_mode: BlendMode::Overlay,
        transform: Transform {
            x: 10_700.0,
            y: 8_350.0,
            rotation: 5.0,
            ..Transform::default()
        },
        mask: LayerMask {
            enabled: true,
            x: 0.07,
            y: 0.08,
            width: 0.86,
            height: 0.84,
            invert: false,
        },
        adjustments: spectrum_imaging::Adjustments {
            exposure: 0.12,
            sharpening: 2.0,
            ..Default::default()
        },
        kind: LayerKind::Raster {
            path: jpeg,
            original_path: None,
        },
        ..Layer::default()
    });
    document.layers.push(Layer {
        id: 3,
        opacity: 0.69,
        blend_mode: BlendMode::SoftLight,
        clip_to_below: true,
        transform: Transform {
            x: 10_950.0,
            y: 8_500.0,
            rotation: -7.0,
            ..Transform::default()
        },
        mask: LayerMask {
            enabled: true,
            x: 0.11,
            y: 0.09,
            width: 0.78,
            height: 0.82,
            invert: true,
        },
        pixel_mask: Some(PixelMask::new(
            2_048,
            1_024,
            (0..2_048_u64 * 1_024)
                .map(|index| if index % 11 == 0 { 128 } else { 255 })
                .collect::<Vec<_>>(),
        )),
        adjustments: spectrum_imaging::Adjustments {
            contrast: 7.0,
            spots: vec![spectrum_imaging::SpotRemoval {
                x: 0.5,
                y: 0.5,
                radius: 0.006,
                opacity: 0.75,
            }],
            ..Default::default()
        },
        kind: LayerKind::Raster {
            path: tiff,
            original_path: None,
        },
        ..Layer::default()
    });
    document
}

struct FixtureResolver {
    sources: HashMap<PathBuf, ResolvedRasterSource>,
}

impl RasterSourceResolver for FixtureResolver {
    fn snapshot_epoch(&self) -> u64 {
        1
    }

    fn resolve(&self, path: &Path) -> Option<ResolvedRasterSource> {
        self.sources.get(path).cloned()
    }
}

struct BenchmarkDirectory(PathBuf);

impl BenchmarkDirectory {
    fn new() -> Result<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "prism-mixed-raster-benchmark-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for BenchmarkDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
