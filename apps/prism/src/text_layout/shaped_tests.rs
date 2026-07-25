use super::*;
use crate::{TextEffects, TextShaping};
use sha2::{Digest, Sha256};

fn shaped_typography() -> TextTypography {
    TextTypography {
        shaping: TextShaping::harfbuzz_v1(Some("en-US")).unwrap(),
        ..Default::default()
    }
}

#[test]
fn shaped_layout_is_deterministic_and_rasterizes_indexed_outlines() {
    let typography = shaped_typography();
    let first = render(
        "office A\u{301}V",
        48.0,
        [230, 220, 200, 255],
        &typography,
        None,
    )
    .unwrap();
    let second = render(
        "office A\u{301}V",
        48.0,
        [230, 220, 200, 255],
        &typography,
        None,
    )
    .unwrap();
    assert_eq!(first, second);
    assert!(first.pixels().any(|pixel| pixel[3] > 0));
}

#[test]
fn mixed_direction_wrapping_and_region_render_share_geometry() {
    let typography = TextTypography {
        box_width: Some(170.0),
        tracking: 1.0,
        shaping: TextShaping::harfbuzz_v1(Some("ar")).unwrap(),
        effects: TextEffects {
            outline_width: 2.0,
            shadow_offset_x: 3.0,
            shadow_offset_y: 4.0,
            shadow_color: [0, 0, 0, 120],
            ..Default::default()
        },
        ..Default::default()
    };
    let text = "Latin العربية office שלום";
    let geometry = measure(text, 36.0, &typography, None).unwrap();
    let full = render(text, 36.0, [240, 230, 210, 255], &typography, None).unwrap();
    assert_eq!((geometry.width, geometry.height), full.dimensions());
    let region = RenderRegion {
        x: 0,
        y: 0,
        width: full.width().min(96),
        height: full.height().min(64),
    };
    let strip = render_region(text, 36.0, [240, 230, 210, 255], &typography, None, region).unwrap();
    assert_eq!(
        strip,
        image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
            .to_image()
    );
}

#[test]
fn long_text_measurement_and_small_region_do_not_stage_the_full_surface() {
    let typography = shaped_typography();
    let long_text = "Cold imported text edit ".repeat(128);
    let geometry = measure(&long_text, 96.0, &typography, None).unwrap();
    assert!(
        u64::from(geometry.width) * u64::from(geometry.height) > MAX_LAYOUT_PIXELS,
        "fixture must exceed the full-surface allocation budget"
    );
    let region = RenderRegion {
        x: 0,
        y: 0,
        width: 192,
        height: geometry.height.min(128),
    };
    let strip = render_region(
        &long_text,
        96.0,
        [240, 230, 210, 255],
        &typography,
        None,
        region,
    )
    .unwrap();
    assert_eq!(strip.dimensions(), (region.width, region.height));
    assert!(strip.pixels().any(|pixel| pixel[3] > 0));

    let manageable = "Cold imported text edit ".repeat(4);
    let full = render(&manageable, 96.0, [240, 230, 210, 255], &typography, None).unwrap();
    let oracle_region = RenderRegion {
        x: 16,
        y: 0,
        width: 160.min(full.width().saturating_sub(16)),
        height: full.height().min(96),
    };
    assert_eq!(
        render_region(
            &manageable,
            96.0,
            [240, 230, 210, 255],
            &typography,
            None,
            oracle_region,
        )
        .unwrap(),
        image::imageops::crop_imm(
            &full,
            oracle_region.x,
            oracle_region.y,
            oracle_region.width,
            oracle_region.height,
        )
        .to_image()
    );
}

#[test]
fn pinned_multiscript_corpus_has_stable_metrics_and_pixels() {
    let typography = shaped_typography();
    let cases = [
        "office ffi AV",
        "العربية لا",
        "हिन्दी",
        "English שלום 123",
        "A\u{301} e\u{308}",
        "👩\u{200d}🎨",
    ];
    let expected = [
        "891a9fb3f078223ec73bedb90f6489ec92441506f70188bd8e4a41cfaab69b52",
        "0970f120330eaa87b83d466502a0aed258f2f8e243eaa4754855425792a6e07a",
        "3798cbea93f27bbbf5aef9957d55a6127bd225dd4293ef7b55023c1ecec0030a",
        "b8886a5fb978fbe9bb25feec6ed3fe6489efd809cbf71100c184c6269d66c05d",
        "1d18c18a34533817956e94b6fd3bf8d4a00e1666d5f55bcaa7e322debab1c44f",
        "fa3bb4c4ed937050585befafb8673f8591a87e3880855f777213ddf7a9595323",
    ];
    let actual = cases
        .into_iter()
        .map(|text| {
            let geometry = measure(text, 40.0, &typography, None).unwrap();
            let image = render(text, 40.0, [231, 219, 197, 255], &typography, None).unwrap();
            let mut digest = Sha256::new();
            digest.update(geometry.width.to_be_bytes());
            digest.update(geometry.height.to_be_bytes());
            digest.update(image.as_raw());
            digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}
