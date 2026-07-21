use std::{
    cell::{Cell, RefCell},
    convert::Infallible,
};

use image::{DynamicImage, Rgba, RgbaImage};

use super::*;
use crate::{CropRect, SpotRemoval};

fn patterned_source(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        Rgba([
            ((x * 17 + y * 3) % 256) as u8,
            ((x * 5 + y * 19) % 256) as u8,
            ((x * 11 + y * 7) % 256) as u8,
            (48 + (x * 7 + y * 13) % 208) as u8,
        ])
    })
}

fn render_region_from_image(
    source: &RgbaImage,
    adjustments: Adjustments,
    region: PixelRegion,
) -> RgbaImage {
    render_image_region_at_source_resolution(
        source.width(),
        source.height(),
        adjustments,
        region,
        |requested| {
            Ok::<_, Infallible>(
                image::imageops::crop_imm(
                    source,
                    requested.x,
                    requested.y,
                    requested.width,
                    requested.height,
                )
                .to_image(),
            )
        },
    )
    .unwrap()
}

#[test]
fn source_resolution_contract_matches_default_options_and_accepts_a_consuming_reader() {
    let source = patterned_source(40, 24);
    let adjustments = Adjustments {
        exposure: 0.25,
        ..Default::default()
    };
    let expected = render_image(
        DynamicImage::ImageRgba8(source.clone()),
        adjustments.clone(),
        RenderOptions::default(),
    )
    .to_rgba8();
    let resized = render_image(
        DynamicImage::ImageRgba8(source.clone()),
        adjustments.clone(),
        RenderOptions { max_size: Some(10) },
    );
    assert_eq!(
        adjusted_image_dimensions(source.width(), source.height(), &adjustments),
        Some(expected.dimensions())
    );
    assert_ne!((resized.width(), resized.height()), expected.dimensions());

    let source_dimensions = source.dimensions();
    let actual = render_image_region_at_source_resolution(
        source_dimensions.0,
        source_dimensions.1,
        adjustments,
        PixelRegion {
            x: 0,
            y: 0,
            width: source_dimensions.0,
            height: source_dimensions.1,
        },
        move |requested| {
            assert_eq!(
                requested,
                PixelRegion {
                    x: 0,
                    y: 0,
                    width: source_dimensions.0,
                    height: source_dimensions.1,
                }
            );
            Ok::<_, Infallible>(source)
        },
    )
    .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn invalid_dimensions_and_regions_fail_before_the_reader_is_invoked() {
    let adjustments = Adjustments::default();
    assert_eq!(adjusted_image_dimensions(0, 8, &adjustments), None);
    assert_eq!(adjusted_image_dimensions(8, 0, &adjustments), None);
    assert_eq!(adjusted_image_dimensions(0, 0, &adjustments), None);

    let cases = [
        (
            0,
            8,
            PixelRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            true,
        ),
        (
            8,
            0,
            PixelRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            true,
        ),
        (
            8,
            6,
            PixelRegion {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            },
            false,
        ),
        (
            8,
            6,
            PixelRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 0,
            },
            false,
        ),
        (
            8,
            6,
            PixelRegion {
                x: u32::MAX,
                y: 0,
                width: 2,
                height: 1,
            },
            false,
        ),
        (
            8,
            6,
            PixelRegion {
                x: 0,
                y: u32::MAX,
                width: 1,
                height: 2,
            },
            false,
        ),
        (
            8,
            6,
            PixelRegion {
                x: 7,
                y: 0,
                width: 2,
                height: 1,
            },
            false,
        ),
        (
            8,
            6,
            PixelRegion {
                x: 0,
                y: 5,
                width: 1,
                height: 2,
            },
            false,
        ),
    ];
    for (source_width, source_height, region, invalid_source) in cases {
        let called = Cell::new(false);
        let error = render_image_region_at_source_resolution(
            source_width,
            source_height,
            adjustments.clone(),
            region,
            |requested| {
                called.set(true);
                Ok::<_, Infallible>(RgbaImage::new(requested.width, requested.height))
            },
        )
        .unwrap_err();
        if invalid_source {
            assert!(matches!(error, RegionRenderError::InvalidSourceDimensions));
        } else {
            assert!(matches!(error, RegionRenderError::InvalidRegion(_)));
        }
        assert!(!called.get(), "reader was invoked for {region:?}");
    }
}

#[test]
fn adjusted_regions_match_full_render_with_geometry_filters_vignette_and_alpha() {
    let source = patterned_source(53, 41);
    let adjustments = Adjustments {
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
        let rendered = render_region_from_image(&source, adjustments.clone(), region);
        let oracle =
            image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
                .to_image();
        assert_eq!(rendered, oracle, "adjusted region {region:?} diverged");
    }
}

#[test]
fn geometry_matrix_matches_full_render_at_interior_and_edges() {
    let source = patterned_source(31, 23);
    for rotation in [0, 90, 180, 270] {
        for (flip_horizontal, flip_vertical) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let adjustments = Adjustments {
                rotation,
                flip_horizontal,
                flip_vertical,
                straighten: -6.25,
                crop: Some(CropRect {
                    x: 0.07,
                    y: 0.13,
                    width: 0.81,
                    height: 0.72,
                }),
                ..Default::default()
            };
            let full = render_image(
                DynamicImage::ImageRgba8(source.clone()),
                adjustments.clone(),
                RenderOptions::default(),
            )
            .to_rgba8();
            assert_eq!(
                full.dimensions(),
                adjusted_image_dimensions(source.width(), source.height(), &adjustments).unwrap()
            );
            let regions = [
                PixelRegion {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                PixelRegion {
                    x: full.width() / 3,
                    y: full.height() / 4,
                    width: (full.width() / 2).max(1),
                    height: (full.height() / 2).max(1),
                },
                PixelRegion {
                    x: full.width() - 1,
                    y: full.height() - 1,
                    width: 1,
                    height: 1,
                },
            ];
            for region in regions {
                let actual = render_region_from_image(&source, adjustments.clone(), region);
                let expected = image::imageops::crop_imm(
                    &full,
                    region.x,
                    region.y,
                    region.width,
                    region.height,
                )
                .to_image();
                assert_eq!(
                    actual, expected,
                    "rotation={rotation}, horizontal={flip_horizontal}, vertical={flip_vertical}, region={region:?}"
                );
            }
        }
    }
}

#[test]
fn adjacent_regions_reconstruct_full_filtered_render_without_seams() {
    let source = patterned_source(67, 49);
    let adjustments = Adjustments {
        temperature: 17.0,
        clarity: 21.0,
        vignette: -31.0,
        noise_reduction: 38.0,
        sharpening: 29.0,
        rotation: 270,
        flip_vertical: true,
        straighten: 4.5,
        crop: Some(CropRect {
            x: 0.04,
            y: 0.08,
            width: 0.88,
            height: 0.84,
        }),
        ..Default::default()
    };
    let full = render_image(
        DynamicImage::ImageRgba8(source.clone()),
        adjustments.clone(),
        RenderOptions::default(),
    )
    .to_rgba8();
    let split_x = full.width() / 2;
    let split_y = full.height() / 2;
    let regions = [
        PixelRegion {
            x: 0,
            y: 0,
            width: split_x,
            height: split_y,
        },
        PixelRegion {
            x: split_x,
            y: 0,
            width: full.width() - split_x,
            height: split_y,
        },
        PixelRegion {
            x: 0,
            y: split_y,
            width: split_x,
            height: full.height() - split_y,
        },
        PixelRegion {
            x: split_x,
            y: split_y,
            width: full.width() - split_x,
            height: full.height() - split_y,
        },
    ];
    let mut reconstructed = RgbaImage::new(full.width(), full.height());
    for region in regions {
        let tile = render_region_from_image(&source, adjustments.clone(), region);
        image::imageops::replace(
            &mut reconstructed,
            &tile,
            i64::from(region.x),
            i64::from(region.y),
        );
    }
    assert_eq!(reconstructed, full);
}

#[test]
fn synthetic_large_source_is_read_once_and_never_in_full() {
    let source_dimensions = (16_384, 12_288);
    let adjustments = Adjustments {
        rotation: 90,
        flip_horizontal: true,
        straighten: 5.0,
        noise_reduction: 18.0,
        sharpening: 12.0,
        crop: Some(CropRect {
            x: 0.05,
            y: 0.08,
            width: 0.9,
            height: 0.84,
        }),
        ..Default::default()
    };
    let requests = RefCell::new(Vec::new());
    let rendered = render_image_region_at_source_resolution(
        source_dimensions.0,
        source_dimensions.1,
        adjustments,
        PixelRegion {
            x: 4_000,
            y: 3_000,
            width: 96,
            height: 72,
        },
        |requested| {
            requests.borrow_mut().push(requested);
            Ok::<_, Infallible>(RgbaImage::from_fn(
                requested.width,
                requested.height,
                |x, y| {
                    let source_x = requested.x + x;
                    let source_y = requested.y + y;
                    Rgba([
                        (source_x % 256) as u8,
                        (source_y % 256) as u8,
                        ((source_x + source_y) % 256) as u8,
                        255,
                    ])
                },
            ))
        },
    )
    .unwrap();
    let requests = requests.into_inner();
    assert_eq!(rendered.dimensions(), (96, 72));
    assert_eq!(requests.len(), 1);
    assert!(
        u64::from(requests[0].width) * u64::from(requests[0].height)
            < u64::from(source_dimensions.0) * u64::from(source_dimensions.1) / 1_000
    );
}

#[test]
fn source_errors_and_wrong_dimensions_are_reported() {
    let region = PixelRegion {
        x: 0,
        y: 0,
        width: 4,
        height: 3,
    };
    let error =
        render_image_region_at_source_resolution(8, 6, Adjustments::default(), region, |_| {
            Err::<RgbaImage, _>("decoder stopped")
        })
        .unwrap_err();
    assert!(matches!(
        error,
        RegionRenderError::Source("decoder stopped")
    ));

    let error =
        render_image_region_at_source_resolution(8, 6, Adjustments::default(), region, |_| {
            Ok::<_, Infallible>(RgbaImage::new(1, 1))
        })
        .unwrap_err();
    assert!(matches!(
        error,
        RegionRenderError::SourceRegionDimensions { .. }
    ));
}

#[test]
fn spots_are_rejected_before_reading_source_pixels() {
    let called = RefCell::new(false);
    let error = render_image_region_at_source_resolution(
        16,
        12,
        Adjustments {
            spots: vec![SpotRemoval {
                x: 0.5,
                y: 0.5,
                radius: 0.1,
                opacity: 1.0,
            }],
            ..Default::default()
        },
        PixelRegion {
            x: 2,
            y: 2,
            width: 4,
            height: 3,
        },
        |requested| {
            *called.borrow_mut() = true;
            Ok::<_, Infallible>(RgbaImage::new(requested.width, requested.height))
        },
    )
    .unwrap_err();
    assert!(matches!(error, RegionRenderError::UnsupportedSpots));
    assert!(!called.into_inner());
}
