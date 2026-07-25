use std::{
    convert::Infallible,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use prism_core::{
    BrushMode, BrushProgram, BrushSample, BrushStroke, BrushStyle, Layer, LayerKind,
    RasterSourceEpoch, ResolvedRasterSource, SampledSourceSnapshot, Transform,
};
use spectrum_imaging::{
    Adjustments, ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness,
    RegionSourceDescriptor, RegionSourceInfo, SourceSampleDepth,
};

use super::*;

struct CloneSource {
    info: RegionSourceInfo,
    pixel: [u8; 4],
    max_read_pixels: Option<Arc<AtomicU64>>,
}

impl ExactRegionSource for CloneSource {
    type Error = Infallible;

    fn info(&self) -> &RegionSourceInfo {
        &self.info
    }

    fn read_exact_region(&self, region: PixelRegion) -> Result<image::RgbaImage, Self::Error> {
        if let Some(max_read_pixels) = &self.max_read_pixels {
            max_read_pixels.fetch_max(
                u64::from(region.width) * u64::from(region.height),
                Ordering::SeqCst,
            );
        }
        Ok(image::RgbaImage::from_pixel(
            region.width,
            region.height,
            image::Rgba(self.pixel),
        ))
    }
}

#[test]
fn hidden_clone_source_uses_authenticated_exact_document_compositor() {
    let source_path = PathBuf::from("hidden-clone-source.png");
    let content_hash = "1b".repeat(32);
    let snapshot = SampledSourceSnapshot {
        version: prism_core::SAMPLED_SOURCE_VERSION,
        source_layer_id: 1,
        source_layer_name: "Hidden source".into(),
        path: source_path.clone(),
        content_hash: content_hash.clone(),
        width: 4,
        height: 4,
        anchor_local: [1.5, 1.5],
        source_transform: Transform::default(),
        adjustments: Adjustments::default(),
        pixel_mask: None,
        vector_mask: None,
    };
    let stroke = BrushStroke::new_clone_stamp(
        BrushStyle {
            mode: BrushMode::CloneStamp,
            color: [0; 4],
            size: 2.0,
            hardness: 1.0,
            opacity: 1.0,
            spacing: 0.25,
        },
        [BrushSample {
            x: 5.5,
            y: 2.5,
            pressure: 1.0,
        }],
        snapshot.clone(),
    )
    .unwrap();
    let source_id = serde_json::to_value(&stroke).unwrap()["source"]["source_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let program = BrushProgram::new(8, 4).unwrap().append(stroke).unwrap();
    let mut document = Document::new("Clone compositor", 8, 4);
    document.background = [0; 4];
    document.layers.push(Layer {
        id: 1,
        visible: false,
        name: "Hidden source".into(),
        kind: LayerKind::Raster {
            path: source_path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    document.layers.push(Layer {
        id: 2,
        name: "Clone".into(),
        kind: LayerKind::Paint { program },
        ..Layer::default()
    });
    document.next_id = 3;
    let mut document_json = serde_json::to_value(document).unwrap();
    document_json["sampled_sources"] = serde_json::json!({(source_id): snapshot});
    let document: Document = serde_json::from_value(document_json).unwrap();

    assert_eq!(document.raster_asset_paths(), vec![source_path.clone()]);
    assert!(document_requires_composite_preview(&document));

    let source = ResolvedRasterSource::new_authenticated(
        RasterSourceEpoch::new("clone-source-v1").unwrap(),
        content_hash,
        Arc::new(CloneSource {
            info: RegionSourceInfo {
                descriptor: RegionSourceDescriptor {
                    width: 4,
                    height: 4,
                    color_encoding: "rgba8".into(),
                    sample_depth: SourceSampleDepth::EightBit,
                    frame_index: 0,
                    page_index: 0,
                    decoder_contract: "clone-test".into(),
                },
                capability: RegionReadCapability::DerivedBacking,
                readiness: RegionReadiness::Ready,
            },
            pixel: [220, 40, 90, 255],
            max_read_pixels: None,
        }),
    )
    .unwrap();
    let sources = RasterSourceSnapshot::with_test_provider(7, source_path, source);
    let geometry = CanvasGeometry {
        canvas: Rect::from_min_size(Pos2::ZERO, Vec2::new(8.0, 4.0)),
        viewport: Rect::from_min_size(Pos2::ZERO, Vec2::new(8.0, 4.0)),
        pixels_per_point: 1.0,
    };
    let key =
        CompositePreviewKey::new_with_sources(1, 1, &document, geometry, 1.0, &sources).unwrap();
    assert_eq!(
        key.raster_mode,
        RasterRenderMode::Provider { snapshot_epoch: 7 }
    );
    let rendered = render_composite_request(&CompositeRenderRequest {
        sequence: 1,
        key,
        raster_sources: Arc::clone(&sources),
    })
    .unwrap()
    .to_rgba8();
    assert!(rendered.pixels().any(|pixel| pixel[0] > 0 && pixel[3] > 0));

    let export = std::env::temp_dir().join(format!(
        "prism-gui-clone-export-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    prism_core::export_document_with_sources(&document, &export, 92, sources.as_ref()).unwrap();
    assert_eq!(image::open(&export).unwrap().to_rgba8(), rendered);
    std::fs::remove_file(export).unwrap();
}

#[test]
fn pending_gui_compositor_fails_once_then_prepared_16k_clone_renders() {
    let source_path = PathBuf::from("logical-16k-clone-source.tiff");
    let content_hash = "4c".repeat(32);
    let snapshot = SampledSourceSnapshot {
        version: prism_core::SAMPLED_SOURCE_VERSION,
        source_layer_id: 1,
        source_layer_name: "Hidden 16K source".into(),
        path: source_path.clone(),
        content_hash: content_hash.clone(),
        width: 16_384,
        height: 16_384,
        anchor_local: [8_192.0, 8_192.0],
        source_transform: Transform::default(),
        adjustments: Adjustments::default(),
        pixel_mask: None,
        vector_mask: None,
    };
    let stroke = BrushStroke::new_clone_stamp(
        BrushStyle {
            mode: BrushMode::CloneStamp,
            color: [0; 4],
            size: 24.0,
            hardness: 1.0,
            opacity: 1.0,
            spacing: 0.25,
        },
        [BrushSample {
            x: 320.0,
            y: 200.0,
            pressure: 1.0,
        }],
        snapshot.clone(),
    )
    .unwrap();
    let source_id = serde_json::to_value(&stroke).unwrap()["source"]["source_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let program = BrushProgram::new(640, 400).unwrap().append(stroke).unwrap();
    let mut document = Document::new("Packaged Clone project", 640, 400);
    document.background = [8, 10, 14, 255];
    document.layers.push(Layer {
        id: 1,
        visible: false,
        name: "Hidden 16K source".into(),
        kind: LayerKind::Raster {
            path: source_path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    document.layers.push(Layer {
        id: 2,
        name: "Clone".into(),
        kind: LayerKind::Paint { program },
        ..Layer::default()
    });
    document.next_id = 3;
    let mut document_json = serde_json::to_value(document).unwrap();
    document_json["version"] = serde_json::json!(11);
    document_json["sampled_sources"] = serde_json::json!({(source_id): snapshot});
    let document: Document = serde_json::from_value(document_json).unwrap();
    let geometry = CanvasGeometry {
        canvas: Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 400.0)),
        viewport: Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 400.0)),
        pixels_per_point: 1.0,
    };

    let pending = RasterSourceSnapshot::empty();
    let pending_key =
        CompositePreviewKey::new_with_sources(9, 1, &document, geometry, 1.0, &pending).unwrap();
    assert_eq!(pending_key.raster_mode, RasterRenderMode::FallbackCapped);
    let pending_error = render_composite_request(&CompositeRenderRequest {
        sequence: 1,
        key: pending_key,
        raster_sources: Arc::new(pending),
    })
    .unwrap_err();
    assert!(
        pending_error
            .contains("Clone Stamp layer previews require their document sampled-source registry"),
        "{pending_error}"
    );

    let max_read_pixels = Arc::new(AtomicU64::new(0));
    let prepared_source = ResolvedRasterSource::new_authenticated(
        RasterSourceEpoch::new("prepared-logical-16k").unwrap(),
        content_hash,
        Arc::new(CloneSource {
            info: RegionSourceInfo {
                descriptor: RegionSourceDescriptor {
                    width: 16_384,
                    height: 16_384,
                    color_encoding: "rgba8".into(),
                    sample_depth: SourceSampleDepth::EightBit,
                    frame_index: 0,
                    page_index: 0,
                    decoder_contract: "logical-16k-clone-regression".into(),
                },
                capability: RegionReadCapability::DerivedBacking,
                readiness: RegionReadiness::Ready,
            },
            pixel: [90, 180, 240, 255],
            max_read_pixels: Some(Arc::clone(&max_read_pixels)),
        }),
    )
    .unwrap();
    let prepared = RasterSourceSnapshot::with_test_provider(2, source_path, prepared_source);
    let prepared_key =
        CompositePreviewKey::new_with_sources(9, 1, &document, geometry, 1.0, &prepared).unwrap();
    assert_eq!(
        prepared_key.raster_mode,
        RasterRenderMode::Provider { snapshot_epoch: 2 }
    );
    let rendered = render_composite_request(&CompositeRenderRequest {
        sequence: 2,
        key: prepared_key,
        raster_sources: prepared,
    })
    .unwrap()
    .to_rgba8();
    assert_eq!(rendered.dimensions(), (640, 400));
    assert!(
        rendered
            .pixels()
            .any(|pixel| pixel.0 == [90, 180, 240, 255])
    );
    assert!(max_read_pixels.load(Ordering::SeqCst) > 0);
    assert!(max_read_pixels.load(Ordering::SeqCst) <= 4_096 * 4_096);
}
