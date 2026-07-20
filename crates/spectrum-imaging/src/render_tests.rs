use super::render::*;
use crate::{ColorGrade, CropRect, CurvePoint, SpotRemoval, ToneCurve, ToneCurves};
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use std::time::Instant;

#[test]
fn exposure_brightens_pixels() {
    let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([32, 32, 32, 255])));
    let rendered = render_image(
        source,
        crate::Adjustments {
            exposure: 1.0,
            ..Default::default()
        },
        RenderOptions::default(),
    );
    assert!(rendered.to_rgba8().get_pixel(0, 0)[0] > 32);
}

#[test]
fn identity_render_preserves_pixels() {
    let source = RgbaImage::from_fn(16, 12, |x, y| {
        Rgba([x as u8 * 11, y as u8 * 13, (x + y) as u8 * 7, 255])
    });
    let rendered = render_image(
        DynamicImage::ImageRgba8(source.clone()),
        crate::Adjustments::default(),
        RenderOptions::default(),
    );
    assert_eq!(rendered.to_rgba8(), source);
}

#[test]
fn hsl_mixer_still_changes_target_color() {
    let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([20, 90, 220, 255])));
    let mut adjustments = crate::Adjustments::default();
    adjustments.hsl.blue.saturation = -80.0;
    let rendered = render_image(source, adjustments, RenderOptions::default()).to_rgba8();
    assert!(rendered.get_pixel(0, 0)[2] < 220);
}

#[test]
fn color_grading_tints_midtones() {
    let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([110, 110, 110, 255])));
    let mut adjustments = crate::Adjustments::default();
    adjustments.color_grading.midtones = ColorGrade {
        hue: 30.0,
        saturation: 80.0,
        luminance: 0.0,
    };
    let rendered = render_image(source, adjustments, RenderOptions::default()).to_rgba8();
    let pixel = rendered.get_pixel(0, 0);
    assert!(pixel[0] > pixel[2]);
}

#[test]
fn spot_removal_repairs_isolated_dust_pixel() {
    let mut source = RgbaImage::from_pixel(21, 21, Rgba([30, 30, 30, 255]));
    source.put_pixel(10, 10, Rgba([245, 245, 245, 255]));
    let rendered = render_image(
        DynamicImage::ImageRgba8(source),
        crate::Adjustments {
            spots: vec![SpotRemoval {
                x: 0.5,
                y: 0.5,
                radius: 0.12,
                opacity: 1.0,
            }],
            ..Default::default()
        },
        RenderOptions::default(),
    )
    .to_rgba8();
    assert!(rendered.get_pixel(10, 10)[0] < 80);
}

#[test]
fn crop_and_rotation_change_dimensions() {
    let source = DynamicImage::new_rgba8(4, 2);
    let rendered = render_image(
        source,
        crate::Adjustments {
            rotation: 90,
            crop: Some(CropRect {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            }),
            ..Default::default()
        },
        RenderOptions::default(),
    );
    assert_eq!(rendered.dimensions(), (1, 4));
}

#[test]
#[ignore = "manual release-mode performance benchmark"]
fn interactive_preview_benchmark() {
    let source = DynamicImage::ImageRgba8(RgbaImage::from_fn(1800, 1200, |x, y| {
        Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255])
    }));
    let adjustments = crate::Adjustments {
        exposure: 0.35,
        contrast: 12.0,
        shadows: 18.0,
        vibrance: 8.0,
        curves: ToneCurves {
            master: ToneCurve {
                points: vec![
                    CurvePoint { x: 0.0, y: 0.0 },
                    CurvePoint { x: 0.4, y: 0.35 },
                    CurvePoint { x: 1.0, y: 1.0 },
                ],
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let iterations = 4;
    let started = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(render_image(
            source.clone(),
            adjustments.clone(),
            RenderOptions::default(),
        ));
    }
    let elapsed = started.elapsed();
    eprintln!(
        "interactive preview: {:.1} ms/frame",
        elapsed.as_secs_f64() * 1000.0 / iterations as f64
    );
}

#[test]
fn red_curve_changes_red_only() {
    let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([128, 128, 128, 255])));
    let mut adjustments = crate::Adjustments::default();
    adjustments.curves.red = ToneCurve {
        points: vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 0.5 }],
    };
    let pixel = render_image(source, adjustments, RenderOptions::default())
        .to_rgba8()
        .get_pixel(0, 0)
        .0;
    assert!(pixel[0] < pixel[1]);
    assert_eq!(pixel[1], pixel[2]);
}

#[test]
fn adjusted_regions_match_full_render_with_geometry_and_spatial_effects() {
    let source = RgbaImage::from_fn(53, 41, |x, y| {
        Rgba([
            ((x * 17 + y * 3) % 256) as u8,
            ((x * 5 + y * 19) % 256) as u8,
            ((x * 11 + y * 7) % 256) as u8,
            (96 + (x * 7 + y * 13) % 160) as u8,
        ])
    });
    let adjustments = crate::Adjustments {
        exposure: 0.37,
        contrast: 14.0,
        vibrance: 9.0,
        vignette: -23.0,
        noise_reduction: 31.0,
        sharpening: 24.0,
        rotation: 90,
        flip_horizontal: true,
        straighten: 7.5,
        crop: Some(CropRect {
            x: 0.08,
            y: 0.11,
            width: 0.79,
            height: 0.73,
        }),
        spots: vec![SpotRemoval {
            x: 0.58,
            y: 0.46,
            radius: 0.09,
            opacity: 0.72,
        }],
        ..Default::default()
    };
    let full = render_image(
        DynamicImage::ImageRgba8(source.clone()),
        adjustments.clone(),
        RenderOptions::default(),
    )
    .to_rgba8();
    let regions = [
        PixelRegion {
            x: 0,
            y: 0,
            width: 9,
            height: 7,
        },
        PixelRegion {
            x: 7,
            y: 9,
            width: 17,
            height: 13,
        },
        PixelRegion {
            x: full.width() - 11,
            y: full.height() - 8,
            width: 11,
            height: 8,
        },
    ];
    for region in regions {
        let rendered = render_image_region(
            source.width(),
            source.height(),
            adjustments.clone(),
            region,
            |x, y| *source.get_pixel(x, y),
        )
        .unwrap();
        let oracle =
            image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
                .to_image();
        assert_eq!(rendered, oracle, "adjusted region {region:?} diverged");
    }
}
