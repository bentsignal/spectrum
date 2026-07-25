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
fn tracking_is_applied_only_between_extended_graphemes() {
    let text = "A\u{301}V";
    let mut tracked = shaped_typography();
    tracked.tracking = 24.0;
    let plain = shaped_typography();
    let fonts = ResolvedFonts::new(None).unwrap();
    let context = ShapingContext::new(text);
    let glyphs = shape_range(text, 0..text.len(), &tracked, &fonts, &context).unwrap();
    assert_eq!(context.span(0..1).unwrap(), 0..1);
    assert_eq!(context.span(1..3).unwrap(), 0..1);
    assert_eq!(
        glyphs
            .iter()
            .map(|glyph| (glyph.graphemes.start, glyph.graphemes.end))
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([(0, 1), (1, 2)])
    );
    let plain_width = position_line(&glyphs, 48.0, 0.0).0;
    let tracked_width = position_line(&glyphs, 48.0, tracked.tracking).0;
    assert!((tracked_width - plain_width - 24.0).abs() < 0.001);
    assert!(
        (measure(text, 48.0, &tracked, None).unwrap().layout_width
            - measure(text, 48.0, &plain, None).unwrap().layout_width
            - 24.0)
            .abs()
            < 0.001
    );

    let full = render(text, 48.0, [240, 220, 190, 255], &tracked, None).unwrap();
    let region = RenderRegion {
        x: 0,
        y: 0,
        width: full.width(),
        height: full.height(),
    };
    assert_eq!(
        render_region(text, 48.0, [240, 220, 190, 255], &tracked, None, region).unwrap(),
        full
    );

    let decomposed = vec![
        RawGlyph {
            face: FaceChoice::Primary,
            glyph_id: 1,
            units_per_em: 1_000,
            x_advance: 600,
            x_offset: 0,
            y_offset: 0,
            ascender: 800,
            descender: -200,
            line_gap: 0,
            graphemes: 0..1,
        },
        RawGlyph {
            face: FaceChoice::Primary,
            glyph_id: 2,
            units_per_em: 1_000,
            x_advance: 0,
            x_offset: -180,
            y_offset: 240,
            ascender: 800,
            descender: -200,
            line_gap: 0,
            graphemes: 0..1,
        },
        RawGlyph {
            face: FaceChoice::Primary,
            glyph_id: 3,
            units_per_em: 1_000,
            x_advance: 600,
            x_offset: 0,
            y_offset: 0,
            ascender: 800,
            descender: -200,
            line_gap: 0,
            graphemes: 1..2,
        },
    ];
    let plain_positions = line_pen_positions(&decomposed, 48.0, 0.0, 0.0);
    let tracked_positions = line_pen_positions(&decomposed, 48.0, 24.0, 0.0);
    let relative_mark_x = |positions: &[f32]| {
        positions[1] + decomposed[1].x_offset as f32
            - (positions[0] + decomposed[0].x_offset as f32)
    };
    assert_eq!(
        relative_mark_x(&plain_positions),
        relative_mark_x(&tracked_positions)
    );
    assert_eq!(decomposed[1].y_offset, 240);
    assert_eq!(tracked_positions[0], plain_positions[0]);
    assert_eq!(tracked_positions[1], plain_positions[1]);
    assert!((tracked_positions[2] - plain_positions[2] - 24.0).abs() < 0.001);
}

#[test]
fn rtl_multi_glyph_graphemes_receive_one_inter_grapheme_tracking_gap() {
    let text = "ש\u{5B0}ל";
    let mut typography = shaped_typography();
    typography.tracking = 19.0;
    let fonts = ResolvedFonts::new(None).unwrap();
    let context = ShapingContext::new(text);
    let glyphs = shape_range(text, 0..text.len(), &typography, &fonts, &context).unwrap();
    assert!(glyphs.iter().any(|glyph| glyph.graphemes == (0..1)));
    let plain = position_line(&glyphs, 52.0, 0.0).0;
    let tracked = position_line(&glyphs, 52.0, typography.tracking).0;
    assert!((tracked - plain - 19.0).abs() < 0.001);
}

#[test]
fn paragraph_prefixes_preserve_visual_rtl_advances_and_reject_ligature_splits() {
    let context = ShapingContext::new("abc");
    let glyph = |advance, graphemes| RawGlyph {
        face: FaceChoice::Primary,
        glyph_id: 1,
        units_per_em: 1_000,
        x_advance: advance,
        x_offset: 0,
        y_offset: 0,
        ascender: 800,
        descender: -200,
        line_gap: 0,
        graphemes,
    };
    let visual_rtl = vec![glyph(300, 2..3), glyph(-50, 1..2), glyph(200, 0..1)];
    let rtl = paragraph_advances(&visual_rtl, 1_000.0, &context).unwrap();
    assert_eq!(rtl.measure(0..3, 0.0, &context).unwrap(), 450.0);
    assert_eq!(rtl.measure(1..3, 0.0, &context).unwrap(), 250.0);

    let spanning_ligature = paragraph_advances(&[glyph(500, 0..2)], 1_000.0, &context).unwrap();
    assert!(
        spanning_ligature
            .measure(0..1, 0.0, &context)
            .unwrap_err()
            .to_string()
            .contains("spanning multiple graphemes")
    );
    assert_eq!(
        spanning_ligature.measure(0..2, 0.0, &context).unwrap(),
        500.0
    );
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
