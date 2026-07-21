use crate::{
    BlendMode, Document, FontAsset, Layer, LayerKind, LayerMask, RenderRegion, ShapeStroke,
    TextAlignment, TextEffects, TextTypography, Transform, document_supports_region_native_zoom,
    render_document_region_scaled, render_document_region_scaled_with_stats,
    render_document_scaled,
};

fn alpha_centroid(image: &image::RgbaImage) -> (f32, f32) {
    let mut weight = 0.0_f64;
    let mut x_sum = 0.0_f64;
    let mut y_sum = 0.0_f64;
    for (x, y, pixel) in image.enumerate_pixels() {
        let alpha = f64::from(pixel[3]);
        weight += alpha;
        x_sum += (f64::from(x) + 0.5) * alpha;
        y_sum += (f64::from(y) + 0.5) * alpha;
    }
    assert!(weight > 0.0);
    ((x_sum / weight) as f32, (y_sum / weight) as f32)
}

#[test]
fn rotated_shape_rendering_keeps_the_live_transform_center() {
    let mut document = Document::new("Centered shape", 320, 260);
    document.background = [0, 0, 0, 0];
    let layer = Layer {
        id: 1,
        transform: Transform {
            x: 100.0,
            y: 80.0,
            rotation: 90.0,
            ..Transform::default()
        },
        kind: LayerKind::Rectangle {
            width: 100,
            height: 40,
            color: [255, 255, 255, 255],
            corner_radius: 0.0,
        },
        ..Layer::default()
    };
    let expected_center = crate::layer_geometry(&layer).unwrap().center;
    document.layers.push(layer);

    let full = render_document_scaled(&document, 1.0).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 0,
        y: 0,
        width: document.width,
        height: document.height,
    };
    let viewport = render_document_region_scaled(&document, 1.0, region)
        .unwrap()
        .to_rgba8();

    assert_eq!(viewport, full);
    for image in [&full, &viewport] {
        let center = alpha_centroid(image);
        assert!(
            (center.0 - expected_center[0]).abs() < 0.001,
            "rotated alpha x centroid {} did not preserve live center {}",
            center.0,
            expected_center[0]
        );
        assert!(
            (center.1 - expected_center[1]).abs() < 0.001,
            "rotated alpha y centroid {} did not preserve live center {}",
            center.1,
            expected_center[1]
        );
    }
}

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

fn temporary_large_grayscale_png(label: &str, width: u32, height: u32) -> std::path::PathBuf {
    use std::io::Write;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("prism-region-{label}-{stamp}.png"));
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let mut stream = writer.stream_writer().unwrap();
    let mut row = vec![0; width as usize];
    for y in 0..height {
        for (x, pixel) in row.iter_mut().enumerate() {
            *pixel = ((x as u32 * 17 + y * 31) % 256) as u8;
        }
        stream.write_all(&row).unwrap();
    }
    stream.finish().unwrap();
    path
}

fn temporary_font(label: &str) -> (std::path::PathBuf, FontAsset) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!("prism-region-{label}-{stamp}.ttf"));
    std::fs::write(&path, epaint_default_fonts::HACK_REGULAR).unwrap();
    let asset = FontAsset::import(71, &path).unwrap();
    (path, asset)
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
                    rotation: 7.0,
                    ..Default::default()
                },
                stroke: ShapeStroke {
                    enabled: true,
                    width: 2.0,
                    color: [236, 221, 142, 224],
                },
                kind: LayerKind::Rectangle {
                    width: 34,
                    height: 27,
                    color: [46, 188, 112, 207],
                    corner_radius: 4.0,
                },
                ..Layer::default()
            },
            Layer {
                id: 2,
                opacity: 0.73,
                blend_mode,
                adjustments: spectrum_imaging::Adjustments {
                    exposure: 0.21,
                    vignette: -12.0,
                    rotation: 90,
                    straighten: 3.5,
                    crop: Some(spectrum_imaging::CropRect {
                        x: 0.04,
                        y: 0.06,
                        width: 0.9,
                        height: 0.86,
                    }),
                    ..Default::default()
                },
                transform: Transform {
                    x: 9.0,
                    y: 7.0,
                    rotation: -11.0,
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
                stroke: ShapeStroke {
                    enabled: true,
                    width: 1.5,
                    color: [83, 214, 231, 190],
                },
                kind: LayerKind::Ellipse {
                    width: 27,
                    height: 22,
                    color: [214, 76, 193, 166],
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
        assert_eq!(viewport, oracle, "region mismatch for {blend_mode:?}");
    }
}

#[test]
fn transformed_vector_region_matches_exact_export_crop() {
    let mut document = Document::new("Vector parity", 64, 48);
    document.background = [28, 39, 57, 173];
    document.layers = vec![
        Layer {
            id: 1,
            opacity: 0.84,
            transform: Transform {
                x: -6.0,
                y: 4.0,
                rotation: -9.0,
                ..Default::default()
            },
            stroke: ShapeStroke {
                enabled: true,
                width: 3.0,
                color: [229, 213, 126, 240],
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
            stroke: ShapeStroke {
                enabled: true,
                width: 2.5,
                color: [65, 218, 231, 204],
            },
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
fn rotated_raster_region_matches_export_without_full_source_staging() {
    let raster_path = temporary_raster("rotated-raster", 37, 29);
    let mut document = Document::new("Raster parity", 128, 96);
    document.background = [22, 37, 53, 179];
    document.layers.push(Layer {
        id: 41,
        opacity: 0.77,
        blend_mode: BlendMode::SoftLight,
        transform: Transform {
            x: 13.0,
            y: 9.0,
            scale_x: 1.7,
            scale_y: 1.35,
            rotation: 27.0,
        },
        mask: LayerMask {
            enabled: true,
            x: 0.08,
            y: 0.13,
            width: 0.82,
            height: 0.71,
            invert: true,
        },
        kind: LayerKind::Raster {
            path: raster_path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    assert!(document_supports_region_native_zoom(&document));
    let full = render_document_scaled(&document, 1.5).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 45,
        y: 35,
        width: 31,
        height: 23,
    };
    let (viewport, stats) =
        render_document_region_scaled_with_stats(&document, 1.5, region).unwrap();
    let oracle = image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
        .to_image();
    let _ = std::fs::remove_file(raster_path);
    assert_eq!(viewport.to_rgba8(), oracle);
    assert!(stats.source_staging_pixels < stats.full_source_pixels);
    assert_ne!(stats.source_staging_pixels, stats.output_pixels);
    assert_eq!(stats.source_staging_bytes, stats.source_staging_pixels * 4);
    assert_eq!(stats.max_source_staging_pixels, stats.source_staging_pixels);
    assert_eq!(stats.fallback_decode_bytes, 0);
    assert_eq!(stats.transformed_surface_pixels, 0);
}

#[test]
fn adjusted_raster_region_matches_export_with_development_and_layer_geometry() {
    let raster_path = temporary_raster("adjusted-raster", 73, 51);
    let mut document = Document::new("Adjusted raster parity", 260, 210);
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
            path: raster_path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    assert!(document_supports_region_native_zoom(&document));
    let full = render_document_scaled(&document, 1.5).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 35,
        y: 24,
        width: 170,
        height: 140,
    };
    let (viewport, stats) =
        render_document_region_scaled_with_stats(&document, 1.5, region).unwrap();
    let oracle = image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
        .to_image();
    let _ = std::fs::remove_file(raster_path);
    assert_eq!(viewport.to_rgba8(), oracle);
    assert!(stats.source_staging_pixels < stats.full_source_pixels);
    assert!(stats.adjusted_staging_pixels > 0);
    assert_eq!(stats.fallback_decode_bytes, 0);
    assert_eq!(stats.transformed_surface_pixels, 0);
}

#[test]
fn rotated_text_region_matches_off_center_visible_pivot_export() {
    let mut document = Document::new("Text pivot parity", 220, 160);
    document.background = [19, 25, 38, 213];
    document.layers.push(Layer {
        id: 42,
        opacity: 0.83,
        blend_mode: BlendMode::Overlay,
        transform: Transform {
            x: 38.0,
            y: 21.0,
            scale_x: 1.25,
            scale_y: 0.9,
            rotation: 33.0,
        },
        kind: LayerKind::Text {
            text: "I\nWide visible glyphs".into(),
            font_size: 29.0,
            color: [237, 196, 91, 224],
            typography: Default::default(),
        },
        ..Layer::default()
    });
    assert!(document_supports_region_native_zoom(&document));
    let geometry = crate::measure_text_geometry("I\nWide visible glyphs", 29.0).unwrap();
    assert_ne!(
        geometry.visual_center(),
        (geometry.width as f32 * 0.5, geometry.height as f32 * 0.5)
    );
    let full = render_document_scaled(&document, 1.75).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 210,
        y: 65,
        width: 47,
        height: 39,
    };
    let (viewport, stats) =
        render_document_region_scaled_with_stats(&document, 1.75, region).unwrap();
    let oracle = image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
        .to_image();
    assert_eq!(viewport.to_rgba8(), oracle);
    assert!(stats.source_staging_pixels < stats.full_source_pixels);
    assert_ne!(stats.source_staging_pixels, stats.output_pixels);
    assert_eq!(stats.transformed_surface_pixels, 0);
}

#[test]
fn imported_typography_effect_region_matches_rotated_export_crop() {
    let (font_path, font) = temporary_font("typography-effects");
    let mut document = Document::new("Typography effect parity", 620, 420);
    document.background = [17, 24, 39, 205];
    document.font_assets.push(font);
    document.layers.push(Layer {
        id: 72,
        opacity: 0.86,
        blend_mode: BlendMode::SoftLight,
        transform: Transform {
            x: 84.0,
            y: 52.0,
            scale_x: 1.15,
            scale_y: 0.92,
            rotation: 19.0,
        },
        adjustments: spectrum_imaging::Adjustments {
            exposure: 0.18,
            contrast: 9.0,
            vignette: -13.0,
            rotation: 90,
            straighten: -3.0,
            crop: Some(spectrum_imaging::CropRect {
                x: 0.03,
                y: 0.05,
                width: 0.92,
                height: 0.88,
            }),
            ..Default::default()
        },
        kind: LayerKind::Text {
            text: "Imported fonts wrap\nwith visible effects".into(),
            font_size: 44.0,
            color: [241, 207, 126, 238],
            typography: TextTypography {
                font_id: Some(71),
                alignment: TextAlignment::Right,
                line_height: 1.55,
                tracking: 2.5,
                box_width: Some(330.0),
                effects: TextEffects {
                    outline_width: 3.0,
                    outline_color: [26, 41, 66, 220],
                    shadow_offset_x: -8.0,
                    shadow_offset_y: 11.0,
                    shadow_color: [4, 7, 13, 150],
                },
            },
        },
        ..Layer::default()
    });
    assert!(document_supports_region_native_zoom(&document));
    let geometry = crate::measure_text_geometry_with_typography(
        "Imported fonts wrap\nwith visible effects",
        44.0,
        match &document.layers[0].kind {
            LayerKind::Text { typography, .. } => typography,
            _ => unreachable!(),
        },
        document.font_assets.first(),
    )
    .unwrap();
    assert_ne!(
        geometry.visual_center(),
        (geometry.width as f32 * 0.5, geometry.height as f32 * 0.5)
    );

    let full = render_document_scaled(&document, 1.5).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 205,
        y: 116,
        width: 123,
        height: 91,
    };
    let (viewport, stats) =
        render_document_region_scaled_with_stats(&document, 1.5, region).unwrap();
    let oracle = image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
        .to_image();
    let _ = std::fs::remove_file(font_path);
    assert_eq!(viewport.to_rgba8(), oracle);
    assert!(stats.source_staging_pixels < stats.full_source_pixels);
    assert!(stats.adjusted_staging_pixels > 0);
    assert_eq!(stats.fallback_decode_bytes, 0);
    assert_eq!(stats.transformed_surface_pixels, 0);
}

#[test]
fn raster_larger_than_legacy_full_source_cap_stages_only_visible_rows() {
    let raster_path = temporary_large_grayscale_png("large-raster", 16_384, 1_025);
    let mut document = Document::new("Large raster staging", 16_384, 2_048);
    document.layers.push(Layer {
        id: 43,
        kind: LayerKind::Raster {
            path: raster_path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    let region = RenderRegion {
        x: 8_000,
        y: 400,
        width: 320,
        height: 180,
    };
    let (rendered, stats) =
        render_document_region_scaled_with_stats(&document, 1.0, region).unwrap();
    let _ = std::fs::remove_file(raster_path);
    assert_eq!((rendered.width(), rendered.height()), (320, 180));
    assert!(stats.full_source_pixels > 4_096 * 4_096);
    assert_eq!(stats.source_staging_pixels, 320 * 180);
    assert_eq!(stats.source_staging_bytes, 320 * 180 * 4);
    assert_eq!(stats.fallback_decode_bytes, 0);
}

#[test]
fn text_larger_than_legacy_full_source_cap_stages_only_visible_glyphs() {
    let mut document = Document::new("Large text staging", 16_384, 1_024);
    document.layers.push(Layer {
        id: 44,
        transform: Transform {
            x: 0.0,
            y: 120.0,
            ..Transform::default()
        },
        kind: LayerKind::Text {
            text: "Bounded viewport text ".repeat(1_000),
            font_size: 48.0,
            color: [238, 202, 117, 255],
            typography: Default::default(),
        },
        ..Layer::default()
    });
    let region = RenderRegion {
        x: 8_000,
        y: 120,
        width: 320,
        height: 80,
    };
    let (rendered, stats) =
        render_document_region_scaled_with_stats(&document, 1.0, region).unwrap();
    assert_eq!((rendered.width(), rendered.height()), (320, 80));
    assert!(stats.full_source_pixels > 4_096 * 4_096);
    assert!(stats.source_staging_pixels <= 320 * 80);
    assert_eq!(stats.source_staging_bytes, stats.source_staging_pixels * 4);
    assert_eq!(stats.fallback_decode_bytes, 0);
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
fn huge_stroked_rotated_shapes_render_without_source_staging() {
    let mut document = Document::new(
        "Guarded fallback",
        crate::MAX_CANVAS_DIMENSION,
        crate::MAX_CANVAS_DIMENSION,
    );
    document.layers.push(Layer {
        id: 9,
        blend_mode: BlendMode::Screen,
        adjustments: spectrum_imaging::Adjustments {
            exposure: 0.24,
            vignette: -15.0,
            noise_reduction: 12.0,
            sharpening: 9.0,
            straighten: 3.0,
            ..Default::default()
        },
        transform: Transform {
            rotation: 11.0,
            ..Transform::default()
        },
        stroke: ShapeStroke {
            enabled: true,
            width: 12.0,
            color: [244, 216, 132, 255],
        },
        kind: LayerKind::Rectangle {
            width: crate::MAX_CANVAS_DIMENSION,
            height: crate::MAX_CANVAS_DIMENSION,
            color: [180, 90, 40, 255],
            corner_radius: 1.0,
        },
        ..Layer::default()
    });
    assert!(document_supports_region_native_zoom(&document));
    let (rendered, stats) = render_document_region_scaled_with_stats(
        &document,
        8.0,
        RenderRegion {
            x: 60_000,
            y: 60_000,
            width: 320,
            height: 180,
        },
    )
    .unwrap();
    assert_eq!((rendered.width(), rendered.height()), (320, 180));
    assert!(stats.source_staging_pixels < 320 * 180);
    assert!(stats.adjusted_staging_pixels < 320 * 180);
    assert_eq!(stats.transformed_surface_pixels, 0);
    assert!(stats.full_source_pixels > 4_096 * 4_096);
}

#[test]
fn spot_adjustments_are_region_native_but_unprepared_non_png_rasters_are_not() {
    let mut spotted = Document::new("Spots", 64, 64);
    spotted.layers.push(Layer {
        adjustments: spectrum_imaging::Adjustments {
            spots: vec![spectrum_imaging::SpotRemoval {
                x: 0.5,
                y: 0.5,
                radius: 0.1,
                opacity: 1.0,
            }],
            ..Default::default()
        },
        kind: LayerKind::Rectangle {
            width: 32,
            height: 24,
            color: [120, 180, 220, 255],
            corner_radius: 0.0,
        },
        ..Layer::default()
    });
    assert!(document_supports_region_native_zoom(&spotted));

    let temporary_png = temporary_raster("unsupported-roi", 32, 24);
    let jpeg_path = temporary_png.with_extension("jpg");
    image::RgbImage::new(32, 24).save(&jpeg_path).unwrap();
    let mut jpeg = Document::new("JPEG", 64, 64);
    jpeg.layers.push(Layer {
        kind: LayerKind::Raster {
            path: jpeg_path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    assert!(!document_supports_region_native_zoom(&jpeg));
    let _ = std::fs::remove_file(temporary_png);
    let _ = std::fs::remove_file(jpeg_path);
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
