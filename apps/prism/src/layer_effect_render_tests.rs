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
        bevel: Some(BevelEmboss::default()),
        satin: Some(Satin::default()),
        gradient_overlay: Some(GradientOverlay {
            gradient: ShapeGradient {
                stops: vec![
                    GradientStop::new(0.0, [255, 200, 0, 255]),
                    GradientStop::new(1.0, [0, 120, 255, 128]),
                ],
                ..ShapeGradient::default()
            },
            blend_mode: BlendMode::Overlay,
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
    assert_eq!(
        serde_json::from_str::<LayerStyle>(&json).unwrap(),
        style.clone()
    );
    let shadow_only = serde_json::json!({"drop_shadow": DropShadow::default()});
    let read: LayerStyle = serde_json::from_value(shadow_only).unwrap();
    assert!(read.stroke.is_none() && read.outer_glow.is_none());
    let mut workspace = Workspace::new(styled(LayerStyle::default()), None);
    workspace
        .execute(Command::SetLayerStyle {
            id: 1,
            style: style.clone(),
        })
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

#[test]
#[ignore = "timing probe; run in release with --nocapture"]
fn effects_timing_probe() {
    let mut document = Document::new("Timing", 1920, 1080);
    document.background = [0, 0, 0, 0];
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            x: 100.0,
            y: 300.0,
            ..Transform::default()
        },
        kind: LayerKind::Text {
            text: "Spring sale!".into(),
            font_size: 220.0,
            color: [255, 255, 255, 255],
            typography: Default::default(),
        },
        ..Layer::default()
    });
    document.next_id = 2;
    let cases = [
        ("none", LayerStyle::default()),
        (
            "shadow",
            LayerStyle {
                drop_shadow: Some(DropShadow::default()),
                ..LayerStyle::default()
            },
        ),
        (
            "stroke",
            LayerStyle {
                stroke: Some(LayerStroke {
                    size: 12.0,
                    ..LayerStroke::default()
                }),
                ..LayerStyle::default()
            },
        ),
        (
            "outer glow",
            LayerStyle {
                outer_glow: Some(Glow {
                    size: 40.0,
                    ..Glow::default()
                }),
                ..LayerStyle::default()
            },
        ),
        (
            "inner glow",
            LayerStyle {
                inner_glow: Some(Glow {
                    size: 40.0,
                    ..Glow::default()
                }),
                ..LayerStyle::default()
            },
        ),
        (
            "bevel",
            LayerStyle {
                bevel: Some(BevelEmboss {
                    size: 12.0,
                    ..BevelEmboss::default()
                }),
                ..LayerStyle::default()
            },
        ),
        (
            "satin",
            LayerStyle {
                satin: Some(Satin::default()),
                ..LayerStyle::default()
            },
        ),
        (
            "all",
            LayerStyle {
                drop_shadow: Some(DropShadow::default()),
                stroke: Some(LayerStroke {
                    size: 12.0,
                    ..LayerStroke::default()
                }),
                outer_glow: Some(Glow {
                    size: 40.0,
                    ..Glow::default()
                }),
                inner_glow: Some(Glow {
                    size: 40.0,
                    ..Glow::default()
                }),
                inner_shadow: Some(DropShadow::default()),
                color_overlay: Some(ColorOverlay::default()),
                bevel: Some(BevelEmboss::default()),
                satin: Some(Satin::default()),
                gradient_overlay: Some(GradientOverlay::default()),
            },
        ),
    ];
    let only = std::env::var("PROBE_CASE").ok();
    let rounds = if only.is_some() { 30 } else { 3 };
    for (name, style) in cases {
        if only.as_deref().is_some_and(|only| only != name) {
            continue;
        }
        document.layers[0].style = style;
        let geometry = document_layer_geometry(&document, &document.layers[0]).unwrap();
        let reach = style_reach(&document.layers[0].style);
        let (x, y) = (
            (geometry.min[0] - reach) * 2.0,
            (geometry.min[1] - reach) * 2.0,
        );
        let (w, h) = (
            (geometry.width() + 2.0 * reach) * 2.0,
            (geometry.height() + 2.0 * reach) * 2.0,
        );
        let area = region(x as u32, y as u32, w as u32, h as u32);
        let start = std::time::Instant::now();
        for _ in 0..rounds {
            render_document_region_scaled(&document, 2.0, area).unwrap();
        }
        println!(
            "{name}: {:.1} ms for {w:.0}x{h:.0}",
            start.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds)
        );
    }
}
