use std::{io::Write, path::PathBuf};

use crate::{
    Document, Layer, LayerKind, RenderRegion, Transform, document_supports_region_native_zoom,
    render_document_region_scaled_with_stats, render_document_scaled,
};

fn temporary_path(label: &str, extension: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-fallback-{label}-{stamp}.{extension}"))
}

fn temporary_raster(label: &str, width: u32, height: u32) -> PathBuf {
    let path = temporary_path(label, "png");
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

fn temporary_rgba16_png(label: &str, width: u32, height: u32) -> PathBuf {
    let path = temporary_path(label, "png");
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder.write_header().unwrap();
    let mut stream = writer.stream_writer().unwrap();
    let mut row = vec![0; width as usize * 8];
    for y in 0..height {
        for x in 0..width {
            let channels = [
                (x * 5_003 + y * 997) as u16,
                (x * 2_003 + y * 7_001) as u16,
                (x * 11_003 + y * 3_001) as u16,
                u16::MAX,
            ];
            for (channel, value) in channels.into_iter().enumerate() {
                let offset = x as usize * 8 + channel * 2;
                row[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
            }
        }
        stream.write_all(&row).unwrap();
    }
    stream.finish().unwrap();
    path
}

fn temporary_interlaced_png(label: &str) -> PathBuf {
    // A 1x1 Adam7 image has the same sole scanline payload as its ordinary PNG.
    let path = temporary_raster(label, 1, 1);
    let mut bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[12..16], b"IHDR");
    bytes[28] = 1;
    let crc = png_crc32(&bytes[12..29]);
    bytes[29..33].copy_from_slice(&crc.to_be_bytes());
    std::fs::write(&path, bytes).unwrap();
    path
}

fn temporary_rgba_header(label: &str, width: u32, height: u32, bit_depth: u8) -> PathBuf {
    let path = temporary_path(label, "png");
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[bit_depth, 6, 0, 0, 0]);
    push_png_chunk(&mut bytes, b"IHDR", &ihdr);
    push_png_chunk(&mut bytes, b"IDAT", &[0x78, 0x9c, 0x03, 0, 0, 0, 0, 1]);
    push_png_chunk(&mut bytes, b"IEND", &[]);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn push_png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let start = output.len() - data.len() - kind.len();
    let crc = png_crc32(&output[start..]);
    output.extend_from_slice(&crc.to_be_bytes());
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[test]
fn fallback_stats_account_for_full_decode_and_transformed_surfaces() {
    let jpeg_path = temporary_path("stats", "jpg");
    image::RgbImage::new(32, 24).save(&jpeg_path).unwrap();
    let interlaced_path = temporary_interlaced_png("interlaced-stats");
    let fixtures = [
        (
            "jpeg",
            jpeg_path.clone(),
            spectrum_imaging::Adjustments::default(),
            32_u64 * 24 * 3,
            1_915,
            7_680,
        ),
        (
            "interlaced PNG",
            interlaced_path.clone(),
            spectrum_imaging::Adjustments::default(),
            4,
            5,
            20,
        ),
    ];

    for (
        label,
        path,
        adjustments,
        expected_decode_bytes,
        expected_transformed_pixels,
        expected_peak_bytes,
    ) in fixtures
    {
        let mut document = Document::new(label, 64, 64);
        document.layers.push(Layer {
            adjustments,
            transform: Transform {
                x: 4.0,
                y: 6.0,
                rotation: 13.0,
                ..Default::default()
            },
            kind: LayerKind::Raster {
                path,
                original_path: None,
            },
            ..Layer::default()
        });
        assert!(!document_supports_region_native_zoom(&document), "{label}");
        let region = RenderRegion {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let full = render_document_scaled(&document, 1.0).unwrap().to_rgba8();
        let (viewport, stats) =
            render_document_region_scaled_with_stats(&document, 1.0, region).unwrap();
        assert_eq!(viewport.to_rgba8(), full, "{label}");
        assert_eq!(
            stats.fallback_decode_bytes, expected_decode_bytes,
            "{label}"
        );
        assert_eq!(
            stats.transformed_surface_pixels, expected_transformed_pixels,
            "{label}"
        );
        assert_eq!(stats.fallback_peak_bytes, expected_peak_bytes, "{label}");
    }

    for path in [jpeg_path, interlaced_path] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn adjusted_anisotropic_fallback_is_rejected_before_allocation() {
    let png_path = temporary_raster("bounded-adjusted-spotted", 16_384, 1);
    let jpeg_path = png_path.with_extension("jpg");
    image::RgbImage::new(16_384, 1).save(&jpeg_path).unwrap();
    let fixtures = [(
        "jpeg",
        jpeg_path.clone(),
        spectrum_imaging::Adjustments {
            rotation: 90,
            ..Default::default()
        },
    )];

    for (label, path, adjustments) in fixtures {
        let mut document = Document::new(label, 16_384, 16_384);
        document.layers.push(Layer {
            adjustments,
            transform: Transform {
                scale_x: 0.01,
                scale_y: 100.0,
                rotation: 45.0,
                ..Default::default()
            },
            kind: LayerKind::Raster {
                path,
                original_path: None,
            },
            ..Layer::default()
        });
        let error = render_document_region_scaled_with_stats(
            &document,
            1.0,
            RenderRegion {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        )
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("bounded viewport fallback"),
            "{label}: {error:#}"
        );
    }

    for path in [png_path, jpeg_path] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn rgba16_fallback_peak_is_rejected_from_header_before_decode() {
    let path = temporary_rgba_header("bounded-rgba16", 4_096, 4_096, 16);
    let mut document = Document::new("RGBA16 fallback", 4_096, 4_096);
    document.layers.push(Layer {
        kind: LayerKind::Raster {
            path: path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    assert!(!document_supports_region_native_zoom(&document));
    let error = render_document_region_scaled_with_stats(
        &document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("bounded viewport fallback"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn adjustment_intermediates_reject_near_cap_rgba16_before_decode() {
    let path = temporary_rgba_header("bounded-adjustments", 4_096, 4_096, 16);
    let mut document = Document::new("Adjustment fallback", 4_096, 4_096);
    document.layers.push(Layer {
        adjustments: spectrum_imaging::Adjustments {
            noise_reduction: 20.0,
            sharpening: 20.0,
            spots: vec![spectrum_imaging::SpotRemoval {
                x: 0.5,
                y: 0.5,
                radius: 0.05,
                opacity: 1.0,
            }],
            ..Default::default()
        },
        transform: Transform {
            scale_x: 0.1,
            scale_y: 0.1,
            ..Default::default()
        },
        kind: LayerKind::Raster {
            path: path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    let error = render_document_region_scaled_with_stats(
        &document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("bounded viewport fallback"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn scaled_rgba16_png_uses_fallback_and_matches_export() {
    let path = temporary_rgba16_png("scaled-oracle", 7, 5);
    let mut document = Document::new("RGBA16 oracle", 32, 24);
    document.layers.push(Layer {
        transform: Transform {
            x: 3.0,
            y: 2.0,
            scale_x: 1.4,
            scale_y: 0.8,
            rotation: 17.0,
        },
        kind: LayerKind::Raster {
            path: path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    assert!(!document_supports_region_native_zoom(&document));
    let full = render_document_scaled(&document, 1.5).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 0,
        y: 0,
        width: 48,
        height: 36,
    };
    let (viewport, stats) =
        render_document_region_scaled_with_stats(&document, 1.5, region).unwrap();
    assert_eq!(viewport.to_rgba8(), full);
    assert_eq!(stats.fallback_decode_bytes, 7 * 5 * 8);
    assert_eq!(stats.transformed_surface_pixels, 277);
    assert_eq!(stats.fallback_peak_bytes, 1_360);
    let _ = std::fs::remove_file(path);
}

#[test]
fn truncated_native_png_preserves_decoder_error_sources() {
    let path = temporary_raster("truncated-native", 32, 24);
    let mut bytes = std::fs::read(&path).unwrap();
    let idat = bytes
        .windows(4)
        .position(|window| window == b"IDAT")
        .unwrap();
    let idat_length = u32::from_be_bytes(bytes[idat - 4..idat].try_into().unwrap()) as usize;
    bytes.truncate(idat + 4 + idat_length / 2);
    std::fs::write(&path, bytes).unwrap();

    let mut document = Document::new("Truncated source", 32, 24);
    document.layers.push(Layer {
        adjustments: spectrum_imaging::Adjustments {
            exposure: 0.1,
            ..Default::default()
        },
        kind: LayerKind::Raster {
            path: path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    assert!(document_supports_region_native_zoom(&document));
    let error = render_document_region_scaled_with_stats(
        &document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: 32,
            height: 24,
        },
    )
    .unwrap_err();
    let chain = error.chain().map(ToString::to_string).collect::<Vec<_>>();
    assert!(chain[0].contains("could not read adjusted image source"));
    assert!(
        chain
            .iter()
            .any(|message| message.contains("could not decode PNG row")),
        "{chain:?}"
    );
    assert!(chain.len() >= 3, "{chain:?}");
    let _ = std::fs::remove_file(path);
}
