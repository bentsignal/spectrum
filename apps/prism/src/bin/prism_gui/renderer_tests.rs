use super::*;

#[test]
fn rotated_solid_quad_samples_only_the_font_atlas_white_pixel() {
    let mesh = quad_mesh(
        egui::TextureId::default(),
        Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(160.0, 90.0)),
        None,
        Color32::from_rgb(240, 80, 120),
        23.0,
        None,
    );

    assert_eq!(mesh.vertices.len(), 4);
    assert!(
        mesh.vertices
            .iter()
            .all(|vertex| vertex.uv == egui::epaint::WHITE_UV)
    );
    assert!(
        mesh.vertices
            .iter()
            .all(|vertex| vertex.color == Color32::from_rgb(240, 80, 120))
    );
}

#[test]
fn rotated_texture_quad_preserves_layer_uv_coordinates() {
    let uv = Rect::from_min_max(Pos2::new(0.1, 0.2), Pos2::new(0.8, 0.9));
    let mesh = quad_mesh(
        egui::TextureId::Managed(7),
        Rect::from_min_size(Pos2::ZERO, Vec2::new(160.0, 90.0)),
        Some(uv),
        Color32::WHITE,
        23.0,
        None,
    );

    let actual: Vec<_> = mesh.vertices.iter().map(|vertex| vertex.uv).collect();
    assert_eq!(
        actual,
        vec![
            uv.left_top(),
            uv.right_top(),
            uv.right_bottom(),
            uv.left_bottom()
        ]
    );
}

#[test]
fn text_quad_rotates_about_the_visual_pivot_instead_of_the_raster_center() {
    let rect = Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(120.0, 80.0));
    let pivot = Pos2::new(58.0, 62.0);
    let mesh = quad_mesh(
        egui::TextureId::Managed(7),
        rect,
        Some(Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))),
        Color32::WHITE,
        90.0,
        Some(pivot),
    );

    let first = mesh.vertices[0].pos;
    let expected = pivot + Vec2::new(-(rect.top() - pivot.y), rect.left() - pivot.x);
    assert!(first.distance(expected) < 0.001);
    assert_ne!(pivot, rect.center());
}

#[test]
fn rotated_text_mesh_and_selection_share_one_stable_visual_center() {
    let source = LayerSourceGeometry {
        size: Vec2::new(180.0, 96.0),
        visual_bounds: Rect::from_min_size(Pos2::new(18.0, 24.0), Vec2::new(112.0, 48.0)),
        paragraph_bounds: None,
    };
    let mut layer = Layer {
        transform: Transform {
            x: 70.0,
            y: 45.0,
            scale_x: 1.4,
            scale_y: 0.8,
            ..Transform::default()
        },
        kind: LayerKind::Text {
            text: "Stable pivot".into(),
            font_size: 48.0,
            color: [255; 4],
            typography: prism_core::TextTypography::default(),
        },
        ..Layer::default()
    };
    let unrotated = layer_bounds(&layer, Some(source)).unwrap();
    let pivot = unrotated.center();
    let texture_visual = Rect::from_min_max(Pos2::new(0.12, 0.18), Pos2::new(0.76, 0.82));

    for rotation in [0.0, 23.0, 90.0, 187.0, 359.0] {
        layer.transform.rotation = rotation;
        let selection = rotated_layer_corners(&layer, Some(source)).unwrap();
        let texture_rect = aligned_text_texture_bounds(&layer, source, texture_visual);
        let mesh = quad_mesh(
            egui::TextureId::Managed(7),
            texture_rect,
            Some(Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))),
            Color32::WHITE,
            rotation,
            Some(pivot),
        );
        let positions = mesh
            .vertices
            .iter()
            .map(|vertex| vertex.pos)
            .collect::<Vec<_>>();
        let map_texture_point = |point: Pos2| {
            positions[0]
                + (positions[1] - positions[0]) * point.x
                + (positions[3] - positions[0]) * point.y
        };
        let painted_visual = [
            map_texture_point(texture_visual.left_top()),
            map_texture_point(texture_visual.right_top()),
            map_texture_point(texture_visual.right_bottom()),
            map_texture_point(texture_visual.left_bottom()),
        ];

        for (painted, selected) in painted_visual.into_iter().zip(selection) {
            assert!(painted.distance(selected) < 0.001);
        }
    }
}

#[test]
fn text_texture_alignment_is_stable_across_raster_scale_rounding() {
    let layer = Layer {
        transform: Transform {
            x: 80.0,
            y: 50.0,
            scale_x: 1.5,
            scale_y: 0.75,
            ..Transform::default()
        },
        kind: LayerKind::Text {
            text: "Stable".into(),
            font_size: 48.0,
            color: [255, 255, 255, 255],
            typography: prism_core::TextTypography::default(),
        },
        ..Layer::default()
    };
    let source = LayerSourceGeometry {
        size: Vec2::new(140.0, 80.0),
        visual_bounds: Rect::from_min_size(Pos2::new(12.0, 18.0), Vec2::new(96.0, 42.0)),
        paragraph_bounds: None,
    };
    let expected = Rect::from_min_size(Pos2::new(98.0, 63.5), Vec2::new(144.0, 31.5));

    for texture_visual in [
        Rect::from_min_max(Pos2::new(0.10, 0.20), Pos2::new(0.70, 0.80)),
        Rect::from_min_max(Pos2::new(0.11, 0.21), Pos2::new(0.71, 0.81)),
    ] {
        let texture = aligned_text_texture_bounds(&layer, source, texture_visual);
        let mapped_visual = Rect::from_min_size(
            texture.min
                + Vec2::new(
                    texture_visual.left() * texture.width(),
                    texture_visual.top() * texture.height(),
                ),
            Vec2::new(
                texture_visual.width() * texture.width(),
                texture_visual.height() * texture.height(),
            ),
        );
        assert!(mapped_visual.min.distance(expected.min) < 0.001);
        assert!(mapped_visual.max.distance(expected.max) < 0.001);
        let canonical_visual = Rect::from_min_max(
            Pos2::new(
                source.visual_bounds.left() / source.size.x,
                source.visual_bounds.top() / source.size.y,
            ),
            Pos2::new(
                source.visual_bounds.right() / source.size.x,
                source.visual_bounds.bottom() / source.size.y,
            ),
        );
        let mapped_uv = aligned_text_uv(source, texture_visual, canonical_visual);
        assert!(mapped_uv.min.distance(texture_visual.min) < 0.001);
        assert!(mapped_uv.max.distance(texture_visual.max) < 0.001);
    }
}

#[test]
fn text_visual_key_tracks_typography_but_not_gpu_transform() {
    let layer = Layer {
        kind: LayerKind::Text {
            text: "Typography".into(),
            font_size: 48.0,
            color: [255, 255, 255, 255],
            typography: prism_core::TextTypography::default(),
        },
        ..Layer::default()
    };
    let original = LayerVisualKey::new(&layer, 1.0);

    let mut transformed = layer.clone();
    transformed.transform.x = 240.0;
    transformed.transform.rotation = 27.0;
    assert_eq!(original, LayerVisualKey::new(&transformed, 1.0));

    let mut imported_face = layer.clone();
    let LayerKind::Text { typography, .. } = &mut imported_face.kind else {
        unreachable!();
    };
    typography.font_id = Some(9);
    assert_ne!(original, LayerVisualKey::new(&imported_face, 1.0));

    let mut paragraph = layer.clone();
    let LayerKind::Text { typography, .. } = &mut paragraph.kind else {
        unreachable!();
    };
    typography.tracking = 12.0;
    typography.box_width = Some(320.0);
    assert_ne!(original, LayerVisualKey::new(&paragraph, 1.0));

    let mut effects = layer;
    let LayerKind::Text { typography, .. } = &mut effects.kind else {
        unreachable!();
    };
    typography.effects.outline_width = 4.0;
    assert_ne!(original, LayerVisualKey::new(&effects, 1.0));
}

#[test]
fn shape_visual_key_tracks_gradient_fill_but_not_layer_style() {
    let layer = Layer {
        kind: LayerKind::Rectangle {
            width: 160,
            height: 90,
            color: [40, 80, 120, 255],
            corner_radius: 0.0,
        },
        ..Layer::default()
    };
    let original = LayerVisualKey::new(&layer, 1.0);

    let mut gradient = layer.clone();
    gradient.shape_fill = Some(prism_core::ShapeFill::Gradient(
        prism_core::ShapeGradient::default(),
    ));
    assert_ne!(original, LayerVisualKey::new(&gradient, 1.0));

    let mut styled = layer;
    styled.style.drop_shadow = Some(prism_core::DropShadow::default());
    assert_eq!(original, LayerVisualKey::new(&styled, 1.0));
}
