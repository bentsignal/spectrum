use spectrum_fonts::{
    HarfBuzzShaper, MAX_SHAPE_FEATURES, MAX_SHAPE_SCALARS, MAX_SHAPE_TEXT_BYTES, OpenTypeFeature,
    Script, ShapeRequest, TextDirection,
};
use ttf_parser::Face;

const LAYOUT_TRUE_TYPE: &[u8] = include_bytes!("fonts/noto-sans-layout-source.ttf");
const RICH_TRUE_TYPE: &[u8] = include_bytes!("fonts/noto-sans-rich-rejected.ttf");

#[test]
fn default_features_apply_ligatures_and_gpos_kerning_deterministically() {
    let shaper = HarfBuzzShaper::new(LAYOUT_TRUE_TYPE, 0).expect("layout fixture opens");
    let first = shaper
        .shape(&ShapeRequest::new("ffi"))
        .expect("default ffi shapes");
    let second = shaper
        .shape(&ShapeRequest::new("ffi"))
        .expect("same run shapes twice");
    assert_eq!(first, second);
    assert!(
        first.glyphs().len() < 3,
        "default liga must substitute the fixture ligature"
    );

    let no_ligature = shaper
        .shape(
            &ShapeRequest::new("ffi")
                .features([OpenTypeFeature::global(*b"liga", 0).expect("valid feature")]),
        )
        .expect("explicit liga disable shapes");
    assert_eq!(no_ligature.glyphs().len(), 3);
    let ranged_ligature =
        shaper
            .shape(&ShapeRequest::new("ffiffi").features([
                OpenTypeFeature::for_byte_range(*b"liga", 0, 0..3).expect("valid range"),
            ]))
            .expect("ranged liga override shapes");
    assert_eq!(
        ranged_ligature.glyphs().len(),
        4,
        "only the first ffi sequence has its ligature disabled"
    );

    let kerned = shaper
        .shape(&ShapeRequest::new("AV"))
        .expect("default AV shapes");
    let unkerned = shaper
        .shape(
            &ShapeRequest::new("AV")
                .features([OpenTypeFeature::global(*b"kern", 0).expect("valid feature")]),
        )
        .expect("explicit kern disable shapes");
    assert_eq!(kerned.glyphs().len(), 2);
    assert_eq!(unkerned.glyphs().len(), 2);
    assert_ne!(
        kerned.glyphs()[0].x_advance,
        unkerned.glyphs()[0].x_advance,
        "default GPOS kerning must affect the first advance"
    );
}

#[test]
fn utf8_clusters_and_combining_mark_positioning_are_preserved() {
    let shaper = HarfBuzzShaper::new(RICH_TRUE_TYPE, 0).expect("rich fixture opens");
    let text = "x\u{301}";
    let run = shaper
        .shape(&ShapeRequest::new(text))
        .expect("combining mark shapes");
    assert_eq!(run.source_text_bytes(), 3);
    assert_eq!(run.glyphs().len(), 2);
    assert!(
        run.glyphs()
            .iter()
            .all(|glyph| text.is_char_boundary(glyph.cluster as usize)),
        "every cluster is a UTF-8 byte boundary"
    );
    assert_eq!(
        run.glyphs()
            .iter()
            .map(|glyph| glyph.cluster)
            .collect::<Vec<_>>(),
        vec![0, 1],
        "character-level clusters must report UTF-8 byte offsets"
    );
    assert!(
        run.glyphs()[1].x_offset != 0 || run.glyphs()[1].y_offset != 0,
        "fixture must exercise mark positioning"
    );
}

#[test]
fn explicit_rtl_returns_visual_order_with_utf8_byte_clusters() {
    let shaper = HarfBuzzShaper::new(LAYOUT_TRUE_TYPE, 0).expect("layout fixture opens");
    let run = shaper
        .shape(
            &ShapeRequest::new("ABC")
                .direction(TextDirection::RightToLeft)
                .script(Script::from_iso15924(*b"Latn").expect("valid script"))
                .language("en"),
        )
        .expect("explicit RTL shapes");
    assert_eq!(run.direction(), TextDirection::RightToLeft);
    assert_eq!(run.script().expect("explicit script").iso15924(), *b"Latn");
    assert_eq!(run.language(), "en");
    assert!(!run.direction_was_guessed());
    assert!(!run.script_was_guessed());
    assert!(!run.language_was_defaulted());
    assert_eq!(
        run.glyphs()
            .iter()
            .map(|glyph| glyph.cluster)
            .collect::<Vec<_>>(),
        vec![2, 1, 0]
    );
}

#[test]
fn guessed_properties_are_deterministic_and_locale_independent() {
    let shaper = HarfBuzzShaper::new(LAYOUT_TRUE_TYPE, 0).expect("layout fixture opens");
    let run = shaper
        .shape(&ShapeRequest::new("AV"))
        .expect("guessed run shapes");
    assert_eq!(run.direction(), TextDirection::LeftToRight);
    assert_eq!(run.script().expect("Latin is guessed").iso15924(), *b"Latn");
    assert_eq!(run.language(), "und");
    assert!(run.direction_was_guessed());
    assert!(run.script_was_guessed());
    assert!(run.language_was_defaulted());
    assert_eq!(run.face_index(), 0);
    assert_eq!(
        run.units_per_em(),
        Face::parse(LAYOUT_TRUE_TYPE, 0)
            .expect("fixture parses")
            .units_per_em()
    );
}

#[test]
fn invalid_font_text_language_feature_and_face_inputs_fail_closed() {
    assert!(HarfBuzzShaper::new(b"not a font", 0).is_err());
    assert!(HarfBuzzShaper::new(LAYOUT_TRUE_TYPE, 1).is_err());
    let shaper = HarfBuzzShaper::new(LAYOUT_TRUE_TYPE, 0).expect("layout fixture opens");

    assert!(
        shaper
            .shape(&ShapeRequest::new(""))
            .unwrap_err()
            .to_string()
            .contains("cannot be empty")
    );
    assert!(
        shaper
            .shape(&ShapeRequest::new(&"A".repeat(MAX_SHAPE_TEXT_BYTES + 1)))
            .unwrap_err()
            .to_string()
            .contains("byte length")
    );
    assert!(
        shaper
            .shape(&ShapeRequest::new(&"\u{e9}".repeat(MAX_SHAPE_SCALARS + 1)))
            .unwrap_err()
            .to_string()
            .contains("scalar count")
    );
    assert!(
        shaper
            .shape(&ShapeRequest::new("A").language("en--US"))
            .unwrap_err()
            .to_string()
            .contains("subtags")
    );

    let too_many_features =
        vec![OpenTypeFeature::global(*b"kern", 0).unwrap(); MAX_SHAPE_FEATURES + 1];
    assert!(
        shaper
            .shape(&ShapeRequest::new("A").features(too_many_features))
            .unwrap_err()
            .to_string()
            .contains("feature count")
    );

    let invalid_boundary = OpenTypeFeature::for_byte_range(*b"liga", 0, 1..2).unwrap();
    assert!(
        HarfBuzzShaper::new(RICH_TRUE_TYPE, 0)
            .unwrap()
            .shape(&ShapeRequest::new("\u{e9}").features([invalid_boundary]))
            .unwrap_err()
            .to_string()
            .contains("UTF-8 byte boundaries")
    );
    assert!(OpenTypeFeature::global(*b"BAD!", 1).is_err());
    assert!(Script::from_iso15924(*b"L4tn").is_err());
}
