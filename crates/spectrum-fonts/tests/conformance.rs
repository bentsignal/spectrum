mod support;

use fontdue::{Font, FontSettings};
use sha2::{Digest, Sha256};
use spectrum_fonts::UnicodeVariationSequence;
use spectrum_fonts::{FontSubsetEngine, HarfBuzzSubsetEngine, SubsetRequest};
use ttf_parser::Face;

const STATIC_TRUE_TYPE: &[u8] = include_bytes!("fonts/noto-sans-static-source.ttf");
const LAYOUT_TRUE_TYPE: &[u8] = include_bytes!("fonts/noto-sans-layout-source.ttf");
const RICH_TRUE_TYPE: &[u8] = include_bytes!("fonts/noto-sans-rich-rejected.ttf");
const VARIABLE_TRUE_TYPE: &[u8] = include_bytes!("fonts/noto-sans-variable-rejected.ttf");
const CFF_OPEN_TYPE: &[u8] = include_bytes!("fonts/noto-sans-cff-rejected.otf");
const OUTPUT_GOLDENS: &str = include_str!("fonts/output-goldens.lock");

#[test]
fn checked_fixtures_match_the_locked_golden_hashes() {
    assert_eq!(
        sha256_hex(STATIC_TRUE_TYPE),
        "1de794fb16bb4fc99afef5d597b909791230be4ea216c5762797f09d9d56a04c"
    );
    assert_eq!(
        sha256_hex(LAYOUT_TRUE_TYPE),
        "05549e889a11eb65b542a071e97d71f4333caf5a88408ab10a1ac3de25d4be3a"
    );
    assert_eq!(
        sha256_hex(RICH_TRUE_TYPE),
        "b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5"
    );
    assert_eq!(
        sha256_hex(VARIABLE_TRUE_TYPE),
        "bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d"
    );
    assert_eq!(
        sha256_hex(CFF_OPEN_TYPE),
        "7b8a545d63de82a3325dc3c545b597898c03219bd432b0a18086e7605859c6c4"
    );
}

#[test]
fn bundled_harfbuzz_reduces_static_true_type_deterministically() {
    let request = SubsetRequest::new(0, [u32::from('A'), u32::from('B')], []);
    let first = HarfBuzzSubsetEngine
        .subset(STATIC_TRUE_TYPE, &request)
        .expect("fixture should pass every current runtime guard");
    let second = HarfBuzzSubsetEngine
        .subset(STATIC_TRUE_TYPE, &request)
        .expect("the same fixture and request should remain valid");

    assert_eq!(first.source_bytes, STATIC_TRUE_TYPE.len());
    assert_eq!(first.subset_bytes, first.bytes.len());
    assert!(first.subset_bytes < first.source_bytes);
    assert_eq!(first.bytes, second.bytes);
    assert_requested_mapping_parity(STATIC_TRUE_TYPE, &first.bytes, ['A', 'B']);

    assert_output_golden("static_ab", &first.bytes);
}

#[test]
fn bundled_harfbuzz_preserves_default_and_nondefault_uvs_kinds() {
    let face = Face::parse(STATIC_TRUE_TYPE, 0).expect("static fixture parses");
    let alternate_glyph = face.glyph_index('C').expect("static fixture maps C").0;
    let source = support::with_default_and_nondefault_uvs(STATIC_TRUE_TYPE, alternate_glyph);
    assert_eq!(
        sha256_hex(&source),
        "92de785c4e5dc040a2bbc050c238519a8c49322725d2e8ce8e56328e7837baad",
        "derived UVS source hash is pending"
    );
    let request = SubsetRequest::new(
        0,
        [],
        [
            UnicodeVariationSequence {
                base_codepoint: u32::from('A'),
                selector_codepoint: 0xfe0f,
            },
            UnicodeVariationSequence {
                base_codepoint: u32::from('B'),
                selector_codepoint: 0xfe0f,
            },
        ],
    )
    .with_shaping_samples([vec![u32::from('A'), 0xfe0f], vec![u32::from('B'), 0xfe0f]]);
    let first = HarfBuzzSubsetEngine
        .subset(&source, &request)
        .expect("both cmap14 mapping kinds should pass every runtime guard");
    let second = HarfBuzzSubsetEngine
        .subset(&source, &request)
        .expect("the UVS subset should be deterministic");
    assert_eq!(first.bytes, second.bytes);
    let source_face = Face::parse(&source, 0).expect("derived UVS source parses");
    let output_face = Face::parse(&first.bytes, 0).expect("UVS output parses");
    let source_alternate = source_face
        .glyph_variation_index('B', '\u{fe0f}')
        .expect("source resolves non-default B+VS16");
    let output_alternate = output_face
        .glyph_variation_index('B', '\u{fe0f}')
        .expect("output resolves non-default B+VS16");
    assert_eq!(
        source_face.glyph_hor_advance(source_alternate),
        output_face.glyph_hor_advance(output_alternate)
    );
    assert_eq!(
        source_face.glyph_bounding_box(source_alternate),
        output_face.glyph_bounding_box(output_alternate)
    );
    let source_font =
        Font::from_bytes(source.as_slice(), FontSettings::default()).expect("source loads");
    let output_font =
        Font::from_bytes(first.bytes.as_slice(), FontSettings::default()).expect("output loads");
    assert!(
        !source_font
            .rasterize_indexed(source_alternate.0, 64.0)
            .1
            .is_empty()
    );
    assert!(
        output_font
            .rasterize_indexed(output_alternate.0, 64.0)
            .1
            .is_empty(),
        "fontdue 0.9.3 does not materialize format-14-only alternates; Prism does not yet render UVS alternates"
    );
    assert_output_golden("uvs_default_nondefault", &first.bytes);
}

#[test]
fn uvs_requires_unicode_platform_encoding_five() {
    let face = Face::parse(STATIC_TRUE_TYPE, 0).expect("static fixture parses");
    let alternate_glyph = face.glyph_index('C').expect("static fixture maps C").0;
    let source = support::with_non_unicode_uvs(STATIC_TRUE_TYPE, alternate_glyph);
    let request = SubsetRequest::new(
        0,
        [],
        [UnicodeVariationSequence {
            base_codepoint: u32::from('A'),
            selector_codepoint: 0xfe0f,
        }],
    );
    let error = HarfBuzzSubsetEngine
        .subset(&source, &request)
        .expect_err("format 14 on a non-Unicode encoding record must fail closed");
    assert!(error.to_string().contains("platform 0 encoding 5"));
}

#[test]
fn uvs_requires_a_sorted_cmap_encoding_directory() {
    let face = Face::parse(STATIC_TRUE_TYPE, 0).expect("static fixture parses");
    let alternate_glyph = face.glyph_index('C').expect("static fixture maps C").0;
    let source = support::with_unsorted_uvs_directory(STATIC_TRUE_TYPE, alternate_glyph);
    let request = SubsetRequest::new(
        0,
        [],
        [UnicodeVariationSequence {
            base_codepoint: u32::from('A'),
            selector_codepoint: 0xfe0f,
        }],
    );
    let error = HarfBuzzSubsetEngine
        .subset(&source, &request)
        .expect_err("unsorted cmap encoding records must fail closed");
    assert!(error.to_string().contains("strictly sorted"));
}

#[test]
fn static_true_type_layout_closure_preserves_ligatures_and_positioning() {
    let source = LAYOUT_TRUE_TYPE;
    let source_face = Face::parse(source, 0).expect("layout source parses");
    let source_composite = source_face
        .glyph_index('\u{c5}')
        .expect("layout source maps U+00C5")
        .0;
    assert_eq!(
        support::glyph_contour_count(source, source_composite),
        -1,
        "U+00C5 must remain a structural glyf composite corpus case"
    );
    let request = SubsetRequest::new(
        0,
        [
            u32::from('A'),
            u32::from('V'),
            u32::from('f'),
            u32::from('i'),
            0x00c5,
        ],
        [],
    )
    .with_shaping_samples([
        "AV".chars().map(u32::from).collect::<Vec<_>>(),
        "ffi".chars().map(u32::from).collect::<Vec<_>>(),
    ]);
    let first = HarfBuzzSubsetEngine
        .subset(source, &request)
        .expect("real Noto GSUB/GPOS/GDEF closure should pass shaping parity");
    let second = HarfBuzzSubsetEngine
        .subset(source, &request)
        .expect("layout closure should be deterministic");
    assert_eq!(first.bytes, second.bytes);
    assert_requested_mapping_parity(source, &first.bytes, ['A', 'V', 'f', 'i', '\u{c5}']);
    let output_face = Face::parse(&first.bytes, 0).expect("layout subset parses");
    let output_composite = output_face
        .glyph_index('\u{c5}')
        .expect("layout subset maps U+00C5")
        .0;
    assert_eq!(
        support::glyph_contour_count(&first.bytes, output_composite),
        -1,
        "subsetting must retain U+00C5 as a structural glyf composite"
    );
    assert_output_golden("rich_layout", &first.bytes);
}

#[test]
fn layout_fonts_require_explicit_shaping_samples() {
    let request = SubsetRequest::new(0, [u32::from('A')], []);
    let error = HarfBuzzSubsetEngine
        .subset(LAYOUT_TRUE_TYPE, &request)
        .expect_err("layout closure cannot be claimed without shaping samples");
    assert!(error.to_string().contains("shaping samples"));
}

#[test]
fn unsupported_stat_table_still_fails_closed() {
    let request =
        SubsetRequest::new(0, [u32::from('A')], []).with_shaping_samples([[u32::from('A')]]);
    let error = HarfBuzzSubsetEngine
        .subset(RICH_TRUE_TYPE, &request)
        .expect_err("STAT remains outside the current static envelope");
    assert!(error.to_string().contains("STAT"));
}

#[test]
fn global_true_type_hint_programs_fail_closed_without_interpreter_proof() {
    let source = support::without_tables(RICH_TRUE_TYPE, &[*b"STAT"]);
    let request =
        SubsetRequest::new(0, [u32::from('A')], []).with_shaping_samples([[u32::from('A')]]);
    let error = HarfBuzzSubsetEngine
        .subset(&source, &request)
        .expect_err("hint-bearing fonts require a portable interpreter proof");
    assert!(error.to_string().contains("cvt "));
}

#[test]
fn local_true_type_hint_instructions_fail_closed_without_interpreter_proof() {
    let source = support::without_tables(
        RICH_TRUE_TYPE,
        &[*b"STAT", *b"cvt ", *b"fpgm", *b"gasp", *b"prep"],
    );
    let request =
        SubsetRequest::new(0, [u32::from('A')], []).with_shaping_samples([[u32::from('A')]]);
    let error = HarfBuzzSubsetEngine
        .subset(&source, &request)
        .expect_err("local glyph instructions require a portable interpreter proof");
    assert!(error.to_string().contains("hint instructions"));
}

#[test]
fn real_variable_true_type_is_rejected_before_subsetting() {
    let request = SubsetRequest::new(0, [u32::from('A')], []);
    let error = HarfBuzzSubsetEngine
        .subset(VARIABLE_TRUE_TYPE, &request)
        .expect_err("variable tables must fail closed");
    assert!(error.to_string().contains("not enabled"));
}

#[test]
fn real_cff_open_type_is_rejected_before_subsetting() {
    let request = SubsetRequest::new(0, [u32::from('A')], []);
    let error = HarfBuzzSubsetEngine
        .subset(CFF_OPEN_TYPE, &request)
        .expect_err("CFF-flavored OpenType must fail closed");
    assert!(error.to_string().contains("OTTO"));
}

#[test]
fn malformed_sfnt_corpus_fails_at_the_rust_boundary() {
    let mut out_of_range_table = STATIC_TRUE_TYPE.to_vec();
    out_of_range_table[20..24].copy_from_slice(&u32::MAX.to_be_bytes());

    let mut duplicate_table = STATIC_TRUE_TYPE.to_vec();
    let first_tag = duplicate_table[12..16].to_vec();
    duplicate_table[28..32].copy_from_slice(&first_tag);

    let mut oversized_directory = STATIC_TRUE_TYPE[0..12].to_vec();
    oversized_directory[4..6].copy_from_slice(&129_u16.to_be_bytes());

    let cases = [
        (
            "truncated directory",
            STATIC_TRUE_TYPE[0..12].to_vec(),
            "truncated SFNT data",
        ),
        (
            "out-of-range table",
            out_of_range_table,
            "truncated SFNT data",
        ),
        ("duplicate table", duplicate_table, "duplicate SFNT table"),
        ("oversized directory", oversized_directory, "resource limit"),
    ];
    let request = SubsetRequest::new(0, [u32::from('A')], []);
    for (label, source, expected) in cases {
        let error = match HarfBuzzSubsetEngine.subset(&source, &request) {
            Ok(_) => panic!("{label} reached native subsetting"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(expected),
            "{label} returned unexpected error: {error}"
        );
    }
}

fn assert_requested_mapping_parity<const N: usize>(
    source: &[u8],
    output: &[u8],
    characters: [char; N],
) {
    let source = Face::parse(source, 0).expect("source parses");
    let output = Face::parse(output, 0).expect("output parses");
    for character in characters {
        let source_glyph = source.glyph_index(character).expect("source maps request");
        let output_glyph = output.glyph_index(character).expect("output maps request");
        assert_eq!(
            source.glyph_hor_advance(source_glyph),
            output.glyph_hor_advance(output_glyph),
            "advance changed for U+{:04X}",
            u32::from(character)
        );
        assert_eq!(
            source.glyph_bounding_box(source_glyph),
            output.glyph_bounding_box(output_glyph),
            "bounds changed for U+{:04X}",
            u32::from(character)
        );
    }
}

fn assert_output_golden(case: &str, bytes: &[u8]) {
    let expected = OUTPUT_GOLDENS
        .lines()
        .filter_map(|line| line.split_once('='))
        .find_map(|(name, value)| (name == case).then_some(value))
        .unwrap_or_else(|| panic!("missing produced-output golden for {case}"));
    let actual = sha256_hex(bytes);
    assert_eq!(
        expected.len(),
        64,
        "{case} produced-output golden is pending; approved build produced {actual}"
    );
    assert!(
        expected.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{case} produced-output golden is not hexadecimal"
    );
    assert_eq!(actual, expected, "{case} produced bytes changed");
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
