use std::{
    collections::HashMap,
    error::Error,
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use image::{Rgba, RgbaImage};
use spectrum_imaging::{
    ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness, RegionRequestError,
    RegionSourceDescriptor, RegionSourceInfo, SourceSampleDepth, validate_region_request,
};

use crate::{
    BlendMode, Document, Layer, LayerKind, LayerMask, RasterSourceEpoch, RasterSourceResolver,
    RenderRegion, ResolvedRasterSource, Transform,
    document_supports_region_native_zoom_with_sources, render_document_region_scaled_with_sources,
    render_document_region_scaled_with_sources_and_stats, render_document_scaled,
};

#[derive(Debug)]
enum MemoryReadError {
    Request(RegionRequestError),
    Forced,
}

impl fmt::Display for MemoryReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(error) => error.fmt(formatter),
            Self::Forced => formatter.write_str("forced provider failure"),
        }
    }
}

impl Error for MemoryReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Request(error) => Some(error),
            Self::Forced => None,
        }
    }
}

struct MemorySource {
    info: RegionSourceInfo,
    pixels: RgbaImage,
    fail: bool,
}

impl MemorySource {
    fn new(pixels: RgbaImage, fail: bool) -> Self {
        Self {
            info: RegionSourceInfo {
                descriptor: RegionSourceDescriptor {
                    width: pixels.width(),
                    height: pixels.height(),
                    color_encoding: "rgba8".into(),
                    sample_depth: SourceSampleDepth::EightBit,
                    frame_index: 0,
                    page_index: 0,
                    decoder_contract: "prism-test-memory:v1".into(),
                },
                capability: RegionReadCapability::DerivedBacking,
                readiness: RegionReadiness::Ready,
            },
            pixels,
            fail,
        }
    }
}

impl ExactRegionSource for MemorySource {
    type Error = MemoryReadError;

    fn info(&self) -> &RegionSourceInfo {
        &self.info
    }

    fn read_exact_region(&self, region: PixelRegion) -> Result<RgbaImage, Self::Error> {
        if self.fail {
            return Err(MemoryReadError::Forced);
        }
        validate_region_request(&self.info.descriptor, region, u64::MAX)
            .map_err(MemoryReadError::Request)?;
        Ok(image::imageops::crop_imm(
            &self.pixels,
            region.x,
            region.y,
            region.width,
            region.height,
        )
        .to_image())
    }
}

struct MemoryResolver {
    snapshot_epoch: u64,
    entries: HashMap<PathBuf, ResolvedRasterSource>,
    resolutions: AtomicUsize,
}

impl MemoryResolver {
    fn one(path: PathBuf, snapshot_epoch: u64, source_epoch: &str, source: MemorySource) -> Self {
        let source = ResolvedRasterSource::new(
            RasterSourceEpoch::new(source_epoch).unwrap(),
            Arc::new(source),
        )
        .unwrap();
        Self {
            snapshot_epoch,
            entries: HashMap::from([(path, source)]),
            resolutions: AtomicUsize::new(0),
        }
    }
}

impl RasterSourceResolver for MemoryResolver {
    fn snapshot_epoch(&self) -> u64 {
        self.snapshot_epoch
    }

    fn resolve(&self, path: &Path) -> Option<ResolvedRasterSource> {
        self.resolutions.fetch_add(1, Ordering::Relaxed);
        self.entries.get(path).cloned()
    }
}

fn pixels(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        Rgba([
            ((x * 29 + y * 7) % 256) as u8,
            ((x * 3 + y * 31) % 256) as u8,
            ((x * 17 + y * 11) % 256) as u8,
            (80 + (x * 13 + y * 5) % 176) as u8,
        ])
    })
}

fn opaque_pixels(width: u32, height: u32) -> RgbaImage {
    let mut image = pixels(width, height);
    for pixel in image.pixels_mut() {
        pixel.0[3] = 255;
    }
    image
}

fn raster_document(path: PathBuf, width: u32, height: u32) -> Document {
    let mut document = Document::new("Provider", width, height);
    document.background = [19, 31, 47, 173];
    document.layers.push(Layer {
        id: 1,
        kind: LayerKind::Raster {
            path,
            original_path: None,
        },
        ..Layer::default()
    });
    document
}

#[test]
fn provider_backed_render_never_inspects_its_missing_path() {
    let path = std::env::temp_dir().join(format!(
        "prism-provider-missing-{}-{}.webp",
        std::process::id(),
        91_337
    ));
    let _ = std::fs::remove_file(&path);
    let source_pixels = opaque_pixels(9, 7);
    let resolver = MemoryResolver::one(
        path.clone(),
        4,
        "memory:missing-path:v1",
        MemorySource::new(source_pixels.clone(), false),
    );
    let document = raster_document(path, 9, 7);

    assert!(document_supports_region_native_zoom_with_sources(
        &document, &resolver
    ));
    let rendered = render_document_region_scaled_with_sources(
        &document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: 9,
            height: 7,
        },
        &resolver,
    )
    .unwrap()
    .to_rgba8();
    assert_eq!(rendered, source_pixels);
    assert!(resolver.resolutions.load(Ordering::Relaxed) > 0);
}

#[test]
fn provider_regions_match_the_export_oracle_with_adjustments_and_geometry() {
    let source_pixels = pixels(73, 51);
    let path = std::env::temp_dir().join(format!(
        "prism-provider-parity-{}-{}.png",
        std::process::id(),
        73_051
    ));
    source_pixels.save(&path).unwrap();
    let resolver = MemoryResolver::one(
        path.clone(),
        12,
        "memory:parity:v1",
        MemorySource::new(source_pixels, false),
    );
    let mut document = Document::new("Provider parity", 260, 210);
    document.background = [22, 37, 53, 179];
    document.layers.push(Layer {
        id: 141,
        opacity: 0.77,
        blend_mode: BlendMode::SoftLight,
        adjustments: spectrum_imaging::Adjustments {
            exposure: 0.3,
            vignette: -18.0,
            noise_reduction: 16.0,
            sharpening: 11.0,
            rotation: 90,
            flip_horizontal: true,
            straighten: 5.0,
            crop: Some(spectrum_imaging::CropRect {
                x: 0.07,
                y: 0.09,
                width: 0.82,
                height: 0.76,
            }),
            ..Default::default()
        },
        transform: Transform {
            x: 38.0,
            y: 27.0,
            scale_x: 1.9,
            scale_y: 1.45,
            rotation: 23.0,
        },
        mask: LayerMask {
            enabled: true,
            invert: true,
            x: 0.08,
            y: 0.12,
            width: 0.81,
            height: 0.72,
        },
        kind: LayerKind::Raster {
            path: path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    let full = render_document_scaled(&document, 1.5).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 35,
        y: 24,
        width: 170,
        height: 140,
    };
    let (viewport, stats) =
        render_document_region_scaled_with_sources_and_stats(&document, 1.5, region, &resolver)
            .unwrap();
    let oracle = image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
        .to_image();
    let _ = std::fs::remove_file(path);

    assert_eq!(viewport.to_rgba8(), oracle);
    assert!(stats.adjusted_staging_pixels > 0);
    assert_eq!(stats.fallback_decode_bytes, 0);
    assert_eq!(stats.transformed_surface_pixels, 0);
    let resolved = resolver.entries.values().next().unwrap();
    assert_eq!(resolved.source_epoch().as_str(), "memory:parity:v1");
}

#[test]
fn resolved_provider_failure_never_falls_back_to_a_valid_path() {
    let path = std::env::temp_dir().join(format!(
        "prism-provider-no-fallback-{}-{}.png",
        std::process::id(),
        8_006
    ));
    RgbaImage::from_pixel(8, 6, Rgba([17, 233, 91, 255]))
        .save(&path)
        .unwrap();
    let resolver = MemoryResolver::one(
        path.clone(),
        19,
        "memory:failure:v1",
        MemorySource::new(RgbaImage::from_pixel(8, 6, Rgba([0, 0, 0, 255])), true),
    );
    let document = raster_document(path.clone(), 8, 6);
    let error = render_document_region_scaled_with_sources(
        &document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: 8,
            height: 6,
        },
        &resolver,
    )
    .unwrap_err();
    let _ = std::fs::remove_file(path);

    assert!(format!("{error:#}").contains("forced provider failure"));
}

#[test]
fn missing_provider_is_not_claimed_as_region_native() {
    let path = PathBuf::from("not-present-in-memory-resolver.png");
    let resolver = MemoryResolver {
        snapshot_epoch: 23,
        entries: HashMap::new(),
        resolutions: AtomicUsize::new(0),
    };
    let document = raster_document(path, 8, 6);

    assert!(!document_supports_region_native_zoom_with_sources(
        &document, &resolver
    ));
    assert_eq!(resolver.snapshot_epoch(), 23);
}
