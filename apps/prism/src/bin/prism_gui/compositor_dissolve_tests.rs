use std::sync::Arc;

use super::*;

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
        let rendered = render_composite_request(&CompositeRenderRequest {
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

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
