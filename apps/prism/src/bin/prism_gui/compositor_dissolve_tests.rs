use std::{convert::Infallible, path::PathBuf, sync::Arc};

use super::*;
use prism_core::{RasterSourceEpoch, ResolvedRasterSource};
use spectrum_imaging::{
    ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness, RegionSourceDescriptor,
    RegionSourceInfo, SourceSampleDepth,
};

#[test]
fn direct_dissolve_composite_matches_seeded_move_rotate_resize_oracle() {
    let mut document = Document::new("Transient Dissolve", 8, 7);
    document.background = [7, 11, 13, 255];
    document.layers.push(Layer {
        id: 1,
        opacity: 0.5,
        blend_mode: BlendMode::Dissolve,
        dissolve_seed: 0x1234_5678,
        kind: LayerKind::Rectangle {
            width: 4,
            height: 3,
            color: [230, 80, 150, 255],
            corner_radius: 0.0,
        },
        ..Layer::default()
    });
    let geometry = CanvasGeometry {
        canvas: Rect::from_min_size(Pos2::ZERO, Vec2::new(8.0, 7.0)),
        viewport: Rect::from_min_size(Pos2::ZERO, Vec2::new(8.0, 7.0)),
        pixels_per_point: 1.0,
    };
    let transforms = [
        Transform {
            x: 2.0,
            y: 1.0,
            ..Transform::default()
        },
        Transform {
            x: 2.0,
            y: 1.0,
            rotation: 31.0,
            ..Transform::default()
        },
        Transform {
            x: 1.0,
            y: 2.0,
            scale_x: 1.5,
            scale_y: 0.75,
            ..Transform::default()
        },
    ];
    let mut hashes = Vec::new();
    for transform in transforms {
        document.layers[0].transform = transform;
        let key = CompositePreviewKey::new(1, 0, &document, geometry, 1.0).unwrap();
        let rendered = render_immediate_composite_request(&CompositeRenderRequest {
            sequence: 1,
            key,
            raster_sources: Arc::new(RasterSourceSnapshot::empty()),
        })
        .unwrap()
        .to_rgba8();
        hashes.push(fnv1a64(rendered.as_raw()));
    }
    assert_eq!(
        hashes,
        [
            6_294_322_823_064_834_809,
            8_297_286_732_747_058_165,
            6_748_145_638_372_427_817,
        ]
    );
}

struct PatternSource {
    info: RegionSourceInfo,
}

impl ExactRegionSource for PatternSource {
    type Error = Infallible;

    fn info(&self) -> &RegionSourceInfo {
        &self.info
    }

    fn read_exact_region(&self, region: PixelRegion) -> Result<image::RgbaImage, Self::Error> {
        Ok(image::RgbaImage::from_fn(
            region.width,
            region.height,
            |x, y| {
                let x = region.x + x;
                let y = region.y + y;
                image::Rgba([
                    (x * 17 + y * 3) as u8,
                    (x * 5 + y * 29) as u8,
                    (x * 11 + y * 7) as u8,
                    255,
                ])
            },
        ))
    }
}

#[test]
fn immediate_provider_strips_match_single_region_across_transform_seams() {
    let path = PathBuf::from("provider-pattern.rgba");
    let mut document = Document::new("Provider-backed Dissolve", 13, 11);
    document.background = [3, 5, 7, 255];
    document.layers.push(Layer {
        id: 1,
        kind: LayerKind::Raster {
            path: path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    document.layers.push(Layer {
        id: 2,
        opacity: 0.5,
        blend_mode: BlendMode::Dissolve,
        dissolve_seed: 0x1020_3040,
        kind: LayerKind::Rectangle {
            width: 7,
            height: 5,
            color: [229, 71, 149, 211],
            corner_radius: 0.0,
        },
        ..Layer::default()
    });
    let source = ResolvedRasterSource::new(
        RasterSourceEpoch::new("provider-pattern-epoch").unwrap(),
        Arc::new(PatternSource {
            info: RegionSourceInfo {
                descriptor: RegionSourceDescriptor {
                    width: 13,
                    height: 11,
                    color_encoding: "rgba8".into(),
                    sample_depth: SourceSampleDepth::EightBit,
                    frame_index: 0,
                    page_index: 0,
                    decoder_contract: "test-pattern".into(),
                },
                capability: RegionReadCapability::DerivedBacking,
                readiness: RegionReadiness::Ready,
            },
        }),
    )
    .unwrap();
    let snapshot = RasterSourceSnapshot::with_test_provider(42, path, source);
    let geometry = CanvasGeometry {
        canvas: Rect::from_min_size(Pos2::ZERO, Vec2::new(13.0, 11.0)),
        viewport: Rect::from_min_size(Pos2::ZERO, Vec2::new(13.0, 11.0)),
        pixels_per_point: 1.0,
    };

    for transform in [
        Transform {
            x: 2.0,
            y: 3.0,
            ..Transform::default()
        },
        Transform {
            x: 2.0,
            y: 3.0,
            rotation: 27.0,
            ..Transform::default()
        },
        Transform {
            x: 1.0,
            y: 2.0,
            scale_x: 1.3,
            scale_y: 0.8,
            ..Transform::default()
        },
    ] {
        document.layers[1].transform = transform;
        let key = CompositePreviewKey::new_with_sources(
            7,
            3,
            &document,
            geometry,
            1.0,
            snapshot.as_ref(),
        )
        .unwrap();
        assert_eq!(
            key.raster_mode,
            RasterRenderMode::Provider { snapshot_epoch: 42 }
        );
        let request = CompositeRenderRequest {
            sequence: 1,
            key,
            raster_sources: Arc::clone(&snapshot),
        };
        let single = render_composite_request(&request).unwrap().into_rgba8();
        let immediate = render_immediate_composite_request(&request)
            .unwrap()
            .into_rgba8();
        assert_eq!(immediate, single);
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
