use std::{convert::Infallible, path::PathBuf, sync::Arc};

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
}

impl ExactRegionSource for CloneSource {
    type Error = Infallible;

    fn info(&self) -> &RegionSourceInfo {
        &self.info
    }

    fn read_exact_region(&self, region: PixelRegion) -> Result<image::RgbaImage, Self::Error> {
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
        raster_sources: sources,
    })
    .unwrap()
    .to_rgba8();
    assert!(rendered.pixels().any(|pixel| pixel[0] > 0 && pixel[3] > 0));
}
