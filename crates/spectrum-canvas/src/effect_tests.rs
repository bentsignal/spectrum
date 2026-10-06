use crate::*;

fn styled_shape_document() -> Document {
    let mut document = Document::new("Effects", 80, 50);
    document.background = [0, 0, 0, 0];
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            x: 10.0,
            y: 10.0,
            ..Transform::default()
        },
        style: LayerStyle {
            drop_shadow: Some(DropShadow {
                color: [0, 0, 0, 180],
                offset_x: 20.0,
                offset_y: 0.0,
                blur_radius: 0.0,
            }),
            ..LayerStyle::default()
        },
        shape_fill: Some(ShapeFill::Gradient(ShapeGradient {
            angle: 0.0,
            stops: vec![
                GradientStop::new(0.0, [255, 0, 0, 255]),
                GradientStop::new(1.0, [0, 0, 255, 255]),
            ],
            ..ShapeGradient::default()
        })),
        kind: LayerKind::Rectangle {
            width: 20,
            height: 20,
            color: [255, 255, 255, 255],
            corner_radius: 0.0,
        },
        ..Layer::default()
    });
    document.selected = Some(1);
    document.next_id = 2;
    document
}

#[test]
fn style_and_fill_commands_validate_layer_kind_and_undo_separately() {
    let mut workspace = Workspace::new(Document::new("Commands", 100, 100));
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 40,
            height: 30,
            color: [40, 80, 120, 255],
            corner_radius: 0.0,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetLayerStyle {
            id: 1,
            style: LayerStyle {
                drop_shadow: Some(DropShadow::default()),
                ..LayerStyle::default()
            },
        })
        .unwrap();
    workspace
        .execute(Command::SetShapeFill {
            id: 1,
            fill: Some(ShapeFill::Gradient(ShapeGradient::default())),
        })
        .unwrap();
    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.layer(1).unwrap().shape_fill.is_none());
    assert!(
        workspace
            .document
            .layer(1)
            .unwrap()
            .style
            .drop_shadow
            .is_some()
    );

    workspace
        .execute(Command::AddText {
            text: "No gradient".into(),
            name: None,
            font_size: 24.0,
            color: [255; 4],
            x: 0.0,
            y: 0.0,
            shaping: Default::default(),
        })
        .unwrap();
    assert!(
        workspace
            .execute(Command::SetShapeFill {
                id: 2,
                fill: Some(ShapeFill::Gradient(ShapeGradient::default())),
            })
            .is_err()
    );
}

#[test]
fn gradient_and_shadow_match_full_export_and_exact_region_preview() {
    let document = styled_shape_document();
    let full = render_document_scaled(&document, 1.0).unwrap().to_rgba8();
    let (preview, stats) = render_document_region_scaled_with_stats(
        &document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: document.width,
            height: document.height,
        },
    )
    .unwrap();
    assert_eq!(preview.to_rgba8(), full);
    assert!(full.get_pixel(11, 20)[0] > full.get_pixel(28, 20)[0]);
    assert!(full.get_pixel(28, 20)[2] > full.get_pixel(11, 20)[2]);
    assert!(full.get_pixel(35, 20)[3] > 0, "shadow extends beyond shape");
    assert!(stats.shadow_samples <= stats.output_pixels * 13);
}

#[test]
fn rotated_masked_clipped_shadow_matches_full_export() {
    let mut document = styled_shape_document();
    document.width = 120;
    document.height = 90;
    let mut styled = document.layers.remove(0);
    styled.transform.x = 34.0;
    styled.transform.y = 20.0;
    styled.transform.rotation = 31.0;
    styled.mask = LayerMask {
        enabled: true,
        invert: false,
        x: 0.15,
        y: 0.2,
        width: 0.45,
        height: 0.55,
    };
    styled.blend_mode = BlendMode::Screen;
    styled.clip_to_below = true;
    styled.adjustments = spectrum_imaging::Adjustments {
        exposure: 0.2,
        noise_reduction: 12.0,
        sharpening: 9.0,
        straighten: 2.5,
        ..Default::default()
    };
    document.layers.push(Layer {
        id: 2,
        transform: Transform {
            x: 8.0,
            y: 8.0,
            ..Transform::default()
        },
        kind: LayerKind::Rectangle {
            width: 78,
            height: 62,
            color: [40, 80, 120, 210],
            corner_radius: 6.0,
        },
        ..Layer::default()
    });
    document.layers.push(styled);
    document.next_id = 3;

    let full = render_document_scaled(&document, 1.0).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 12,
        y: 8,
        width: 96,
        height: 74,
    };
    let (preview, stats) =
        render_document_region_scaled_with_stats(&document, 1.0, region).unwrap();
    let preview = preview.to_rgba8();
    let oracle = image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
        .to_image();
    assert_eq!(preview, oracle);
    assert!(stats.adjusted_staging_pixels > 0);
    assert!(stats.shadow_alpha_tile_pixels > 0);
    assert_eq!(
        stats.shadow_alpha_tile_bytes,
        stats.shadow_alpha_tile_pixels
    );
    assert!(stats.max_shadow_alpha_tile_pixels <= 4_096 * 4_096);
    assert_eq!(
        stats.shadow_source_samples, stats.shadow_alpha_tile_pixels,
        "adjusted shadows must not fall back to direct source-alpha sampling"
    );
}

#[test]
fn huge_gradient_shadow_preview_stays_viewport_bounded() {
    let mut document = styled_shape_document();
    document.width = MAX_CANVAS_DIMENSION;
    document.height = MAX_CANVAS_DIMENSION;
    document.layers[0].kind = LayerKind::Rectangle {
        width: MAX_CANVAS_DIMENSION,
        height: MAX_CANVAS_DIMENSION,
        color: [255; 4],
        corner_radius: 0.0,
    };
    document.layers[0]
        .style
        .drop_shadow
        .as_mut()
        .unwrap()
        .blur_radius = 24.0;
    let (_, stats) = render_document_region_scaled_with_stats(
        &document,
        8.0,
        RenderRegion {
            x: 20_000,
            y: 20_000,
            width: 320,
            height: 180,
        },
    )
    .unwrap();
    assert_eq!(stats.source_staging_pixels, 0);
    assert_eq!(stats.transformed_surface_pixels, 0);
    assert!(stats.shadow_samples <= stats.output_pixels * 13);
    assert!(stats.shadow_alpha_tile_pixels > 0);
    assert_eq!(
        stats.shadow_alpha_tile_bytes,
        stats.shadow_alpha_tile_pixels
    );
    assert!(stats.max_shadow_alpha_tile_pixels <= 4_096 * 4_096);
    assert_eq!(
        stats.max_shadow_alpha_tile_bytes,
        stats.max_shadow_alpha_tile_pixels
    );
}
