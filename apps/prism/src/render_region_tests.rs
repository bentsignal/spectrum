use crate::{
    BlendMode, Document, Layer, LayerKind, LayerMask, RenderRegion, ShapeStroke, Transform,
    document_supports_region_native_zoom, render_document_region_scaled,
    render_document_region_scaled_with_stats, render_document_scaled,
};

fn temporary_raster(label: &str, width: u32, height: u32) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("prism-region-{label}-{stamp}.png"));
    image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([
            ((x * 29 + y * 7) % 256) as u8,
            ((x * 3 + y * 31) % 256) as u8,
            ((x * 17 + y * 11) % 256) as u8,
            (80 + (x * 13 + y * 5) % 176) as u8,
        ])
    })
    .save(&path)
    .unwrap();
    path
}

#[test]
fn viewport_regions_match_the_export_oracle_for_every_blend_mode() {
    for (index, blend_mode) in BlendMode::ALL.into_iter().enumerate() {
        let mut document = Document::new("Region parity", 48, 36);
        document.background = [31, 47, 73, 181];
        document.layers = vec![
            Layer {
                id: 1,
                transform: Transform {
                    x: -4.0,
                    y: -3.0,
                    ..Default::default()
                },
                kind: LayerKind::Rectangle {
                    width: 34,
                    height: 27,
                    color: [46, 188, 112, 207],
                    corner_radius: 0.0,
                },
                ..Layer::default()
            },
            Layer {
                id: 2,
                opacity: 0.73,
                blend_mode,
                transform: Transform {
                    x: 9.0,
                    y: 7.0,
                    ..Default::default()
                },
                mask: LayerMask {
                    enabled: true,
                    x: 0.1,
                    y: 0.15,
                    width: 0.72,
                    height: 0.68,
                    invert: index % 2 == 0,
                },
                clip_to_below: index % 3 == 0,
                kind: LayerKind::Rectangle {
                    width: 27,
                    height: 22,
                    color: [214, 76, 193, 166],
                    corner_radius: 0.0,
                },
                ..Layer::default()
            },
        ];
        let full = render_document_scaled(&document, 1.5).unwrap().to_rgba8();
        let region = RenderRegion {
            x: 8,
            y: 6,
            width: 41,
            height: 32,
        };
        let viewport = render_document_region_scaled(&document, 1.5, region)
            .unwrap()
            .to_rgba8();
        let oracle =
            image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
                .to_image();
        if viewport != oracle {
            let first = viewport
                .pixels()
                .zip(oracle.pixels())
                .enumerate()
                .find(|(_, (actual, expected))| actual != expected)
                .unwrap();
            panic!(
                "region mismatch for {blend_mode:?} at ({}, {}): {:?} != {:?}",
                first.0 as u32 % region.width,
                first.0 as u32 / region.width,
                first.1.0,
                first.1.1
            );
        }
    }
}

#[test]
fn transformed_region_matches_exact_export_crop() {
    let mut document = Document::new("Transform parity", 64, 48);
    document.background = [28, 39, 57, 173];
    document.layers = vec![
        Layer {
            id: 1,
            opacity: 0.84,
            transform: Transform {
                x: -6.0,
                y: 4.0,
                ..Default::default()
            },
            kind: LayerKind::Rectangle {
                width: 45,
                height: 34,
                color: [62, 190, 118, 211],
                corner_radius: 5.0,
            },
            ..Layer::default()
        },
        Layer {
            id: 2,
            opacity: 0.69,
            blend_mode: BlendMode::Overlay,
            transform: Transform {
                x: 11.0,
                y: 7.0,
                scale_x: 1.2,
                scale_y: 0.9,
                rotation: 17.0,
            },
            mask: LayerMask {
                enabled: true,
                x: 0.12,
                y: 0.18,
                width: 0.7,
                height: 0.64,
                invert: true,
            },
            clip_to_below: true,
            kind: LayerKind::Ellipse {
                width: 31,
                height: 25,
                color: [219, 78, 187, 196],
            },
            ..Layer::default()
        },
    ];
    assert!(document_supports_region_native_zoom(&document));
    let full = render_document_scaled(&document, 1.5).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 7,
        y: 5,
        width: 53,
        height: 39,
    };
    let viewport = render_document_region_scaled(&document, 1.5, region)
        .unwrap()
        .to_rgba8();
    let oracle = image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
        .to_image();
    assert_eq!(viewport, oracle);
}

#[test]
fn high_zoom_region_is_bounded_by_the_viewport_not_the_document() {
    let mut document = Document::new(
        "Large region",
        crate::MAX_CANVAS_DIMENSION,
        crate::MAX_CANVAS_DIMENSION,
    );
    document.layers.push(Layer {
        id: 1,
        blend_mode: BlendMode::Multiply,
        transform: Transform {
            x: 20.0,
            y: 30.0,
            ..Default::default()
        },
        kind: LayerKind::Rectangle {
            width: 32,
            height: 24,
            color: [120, 180, 220, 210],
            corner_radius: 2.0,
        },
        ..Layer::default()
    });
    let region = RenderRegion {
        x: 128,
        y: 192,
        width: 320,
        height: 180,
    };
    let viewport = render_document_region_scaled(&document, 8.0, region)
        .unwrap()
        .to_rgba8();
    assert_eq!(viewport.dimensions(), (320, 180));
    assert!(render_document_scaled(&document, 8.0).is_err());
}

#[test]
fn huge_rounded_layers_render_without_a_full_source_allocation_failure() {
    let mut document = Document::new(
        "Guarded fallback",
        crate::MAX_CANVAS_DIMENSION,
        crate::MAX_CANVAS_DIMENSION,
    );
    document.layers.push(Layer {
        id: 9,
        blend_mode: BlendMode::Screen,
        kind: LayerKind::Rectangle {
            width: crate::MAX_CANVAS_DIMENSION,
            height: crate::MAX_CANVAS_DIMENSION,
            color: [180, 90, 40, 255],
            corner_radius: 1.0,
        },
        ..Layer::default()
    });
    assert!(document_supports_region_native_zoom(&document));
    let rendered = render_document_region_scaled(
        &document,
        8.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: 320,
            height: 180,
        },
    )
    .unwrap()
    .to_rgba8();
    assert_eq!(rendered.dimensions(), (320, 180));
}

#[test]
fn raster_and_text_regions_match_the_export_oracle_after_transform_and_adjustment() {
    let raster_path = temporary_raster("parity", 29, 23);
    let mut document = Document::new("Mixed source parity", 160, 120);
    document.background = [18, 27, 41, 211];
    document.layers = vec![
        Layer {
            id: 1,
            opacity: 0.81,
            transform: Transform {
                x: 7.0,
                y: 9.0,
                scale_x: 2.3,
                scale_y: 1.7,
                rotation: 21.0,
            },
            adjustments: spectrum_imaging::Adjustments {
                exposure: 0.28,
                contrast: 11.0,
                vignette: -14.0,
                sharpening: 13.0,
                ..Default::default()
            },
            kind: LayerKind::Raster {
                path: raster_path.clone(),
                original_path: None,
            },
            ..Layer::default()
        },
        Layer {
            id: 2,
            opacity: 0.72,
            blend_mode: BlendMode::SoftLight,
            transform: Transform {
                x: 31.0,
                y: 25.0,
                scale_x: 1.25,
                scale_y: 0.88,
                rotation: 347.0,
            },
            mask: LayerMask {
                enabled: true,
                x: 0.08,
                y: 0.13,
                width: 0.81,
                height: 0.69,
                invert: true,
            },
            clip_to_below: true,
            kind: LayerKind::Text {
                text: "Viewport\ntext".into(),
                font_size: 18.0,
                color: [231, 192, 84, 218],
            },
            ..Layer::default()
        },
    ];
    let full = render_document_scaled(&document, 1.75).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 19,
        y: 17,
        width: 121,
        height: 93,
    };
    let viewport = render_document_region_scaled(&document, 1.75, region)
        .unwrap()
        .to_rgba8();
    let oracle = image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
        .to_image();
    let _ = std::fs::remove_file(raster_path);
    assert_eq!(viewport, oracle);
}

#[test]
fn high_zoom_all_layer_sources_report_no_transformed_surface_allocation() {
    let raster_path = temporary_raster("allocation", 32, 24);
    let mut document = Document::new("Allocation accounting", 16_384, 16_384);
    document.layers = vec![
        Layer {
            id: 1,
            transform: Transform {
                scale_x: 100.0,
                scale_y: 100.0,
                rotation: 9.0,
                ..Default::default()
            },
            kind: LayerKind::Raster {
                path: raster_path.clone(),
                original_path: None,
            },
            ..Layer::default()
        },
        Layer {
            id: 2,
            transform: Transform {
                x: 4_000.0,
                y: 4_000.0,
                scale_x: 4.0,
                scale_y: 4.0,
                rotation: 17.0,
            },
            kind: LayerKind::Text {
                text: "Bounded text".into(),
                font_size: 24.0,
                color: [240, 220, 170, 255],
            },
            ..Layer::default()
        },
        Layer {
            id: 3,
            opacity: 0.77,
            blend_mode: BlendMode::Overlay,
            transform: Transform {
                rotation: 23.0,
                ..Default::default()
            },
            adjustments: spectrum_imaging::Adjustments {
                exposure: 0.35,
                vignette: -18.0,
                noise_reduction: 20.0,
                sharpening: 15.0,
                straighten: 4.0,
                ..Default::default()
            },
            stroke: ShapeStroke {
                enabled: true,
                width: 12.0,
                color: [245, 210, 120, 255],
            },
            kind: LayerKind::Ellipse {
                width: 16_384,
                height: 16_384,
                color: [80, 150, 220, 214],
            },
            ..Layer::default()
        },
    ];
    let region = RenderRegion {
        x: 76_000,
        y: 76_000,
        width: 96,
        height: 72,
    };
    let (rendered, stats) = render_document_region_scaled_with_stats(&document, 8.0, region)
        .expect("all layer sources must render at requested high zoom");
    let _ = std::fs::remove_file(raster_path);
    assert_eq!((rendered.width(), rendered.height()), (96, 72));
    assert_eq!(stats.output_pixels, 96 * 72);
    assert_eq!(stats.transformed_surface_pixels, 0);
    assert!(stats.source_staging_pixels < 16_000_000);
    assert!(stats.max_adjusted_tile_pixels <= 64 * 64);
}

#[test]
fn oversized_region_is_rejected_before_canvas_allocation() {
    let document = Document::new("Bounded tile", 8_192, 8_192);
    let error = render_document_region_scaled(
        &document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: 4_097,
            height: 4_097,
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("bounded viewport area"));
}

#[test]
fn translucent_background_is_composited_exactly_once_in_a_region() {
    let mut document = Document::new("Alpha", 32, 24);
    document.background = [80, 120, 160, 128];
    document.layers.push(Layer {
        blend_mode: BlendMode::Multiply,
        opacity: 0.0,
        ..Layer::default()
    });
    let region = render_document_region_scaled(
        &document,
        4.0,
        RenderRegion {
            x: 17,
            y: 23,
            width: 9,
            height: 7,
        },
    )
    .unwrap()
    .to_rgba8();
    assert!(region.pixels().all(|pixel| pixel.0 == document.background));
}
