use prism_core::{Document, render_document, render_document_scaled};
use serde::Deserialize;

#[derive(Deserialize)]
struct BlendParityFixture {
    document: Document,
    expected_rgba: Vec<u8>,
}

#[test]
fn multilayer_blend_mask_and_clipping_match_visual_fixture() {
    let fixture: BlendParityFixture =
        serde_json::from_str(include_str!("fixtures/blend-parity.json")).unwrap();
    let rendered = render_document(&fixture.document, None).unwrap().to_rgba8();
    assert_eq!(
        rendered.as_raw(),
        &fixture.expected_rgba,
        "update the fixture only after visually reviewing an intentional compositor change"
    );
    assert_eq!(
        render_document_scaled(&fixture.document, 1.0)
            .unwrap()
            .to_rgba8(),
        rendered
    );
}

#[test]
fn explicit_preview_scale_is_validated() {
    let document = Document::new("Scale", 4, 4);
    assert!(render_document_scaled(&document, 0.0).is_err());
    assert!(render_document_scaled(&document, f32::NAN).is_err());
    assert_eq!(render_document_scaled(&document, 2.0).unwrap().width(), 8);
}
