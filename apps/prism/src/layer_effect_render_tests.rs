use crate::*;

fn styled(style: LayerStyle) -> Document {
    let mut document = Document::new("Styles", 80, 50);
    document.background = [0, 0, 0, 255];
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            x: 20.0,
            y: 15.0,
            ..Transform::default()
        },
        style,
        kind: LayerKind::Rectangle {
            width: 20,
            height: 20,
            color: [255, 255, 255, 255],
            corner_radius: 0.0,
        },
        ..Layer::default()
    });
    document.next_id = 2;
    document
}

fn region(x: u32, y: u32, width: u32, height: u32) -> RenderRegion {
    RenderRegion {
        x,
        y,
        width,
        height,
    }
}

fn every_style() -> LayerStyle {
    LayerStyle {
        outer_glow: Some(Glow {
            color: [0, 255, 0, 255],
            size: 6.0,
            spread: 0.2,
        }),
        color_overlay: Some(ColorOverlay {
            color: [0, 0, 255, 128],
        }),
        inner_glow: Some(Glow::default()),
        inner_shadow: Some(DropShadow::default()),
        stroke: Some(LayerStroke {
            size: 3.0,
            position: StrokePosition::Outside,
            color: [255, 0, 0, 255],
        }),
        ..LayerStyle::default()
    }
}

#[test]
fn styles_draw_where_photoshop_draws_them() {
    let full = render_document_scaled(&styled(every_style()), 1.0)
        .unwrap()
        .to_rgba8();
    assert_eq!(full.get_pixel(18, 25).0, [255, 0, 0, 255], "stroke outside");
    assert!(
        full.get_pixel(16, 25)[1] > full.get_pixel(12, 25)[1],
        "glow fades out"
    );
    assert_eq!(full.get_pixel(4, 25).0, [0, 0, 0, 255], "past the glow");
    let center = full.get_pixel(30, 25).0;
    assert!(center[2] > center[0], "overlay tints the layer: {center:?}");
}

#[test]
fn styled_regions_match_the_full_render() {
    let document = styled(every_style());
    let full = render_document_scaled(&document, 1.0).unwrap().to_rgba8();
    let whole = render_document_region_scaled(&document, 1.0, region(0, 0, 80, 50))
        .unwrap()
        .to_rgba8();
    assert_eq!(whole, full);
    for tile in [
        region(0, 0, 31, 27),
        region(31, 27, 49, 23),
        region(12, 20, 9, 9),
    ] {
        let part = render_document_region_scaled(&document, 1.0, tile)
            .unwrap()
            .to_rgba8();
        let expected =
            image::imageops::crop_imm(&full, tile.x, tile.y, tile.width, tile.height).to_image();
        assert_eq!(part, expected, "tile {tile:?}");
    }
}

#[test]
fn style_round_trips_and_old_layers_read_without_styles() {
    let style = every_style();
    let json = serde_json::to_string(&style).unwrap();
    assert_eq!(serde_json::from_str::<LayerStyle>(&json).unwrap(), style);
    let shadow_only = serde_json::json!({"drop_shadow": DropShadow::default()});
    let read: LayerStyle = serde_json::from_value(shadow_only).unwrap();
    assert!(read.stroke.is_none() && read.outer_glow.is_none());
    let mut workspace = Workspace::new(styled(LayerStyle::default()), None);
    workspace
        .execute(Command::SetLayerStyle { id: 1, style })
        .unwrap();
    assert_eq!(workspace.document.layers[0].style, style);
    let bad = LayerStyle {
        stroke: Some(LayerStroke {
            size: f32::NAN,
            ..LayerStroke::default()
        }),
        ..LayerStyle::default()
    };
    assert!(
        workspace
            .execute(Command::SetLayerStyle { id: 1, style: bad })
            .is_err()
    );
}
