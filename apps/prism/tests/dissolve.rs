use std::fs;

use prism_core::{
    BlendMode, Command, Document, LayerTransfer, RenderRegion, Workspace, render_document,
    render_document_region_scaled,
};
use serde::Deserialize;
use spectrum_revisions::{Actor, ActorKind, RevisionId, SessionId};

#[derive(Deserialize)]
struct DissolveFixture {
    document: Document,
    expected_rgba: Vec<u8>,
}

fn fixture() -> DissolveFixture {
    serde_json::from_str(include_str!("fixtures/dissolve-parity.json")).unwrap()
}

#[test]
fn seeded_dissolve_matches_the_reviewed_visual_fixture_and_region_crop() {
    let fixture = fixture();
    let rendered = render_document(&fixture.document, None).unwrap().to_rgba8();
    assert_eq!(rendered.as_raw(), &fixture.expected_rgba);

    let region = RenderRegion {
        x: 2,
        y: 1,
        width: 5,
        height: 3,
    };
    let viewport = render_document_region_scaled(&fixture.document, 1.0, region)
        .unwrap()
        .to_rgba8();
    assert_eq!(
        viewport,
        image::imageops::crop_imm(&rendered, region.x, region.y, region.width, region.height)
            .to_image()
    );
}

#[test]
fn seed_command_changes_the_pattern_and_round_trips_durably() {
    let directory = std::env::temp_dir().join(format!("prism-dissolve-{}", RevisionId::new()));
    fs::create_dir_all(&directory).unwrap();
    let project = directory.join("seeded.prism");
    let actor = Actor {
        id: "test:dissolve".into(),
        display_name: "Dissolve test".into(),
        kind: ActorKind::Human,
    };
    let session = SessionId::new();
    let mut workspace =
        Workspace::create_durable(fixture().document, &project, actor.clone(), session).unwrap();
    let before = render_document(&workspace.document, None).unwrap();
    workspace
        .execute(Command::SetDissolveSeed {
            id: 1,
            seed: 0x8765_4321,
        })
        .unwrap();
    let after = render_document(&workspace.document, None).unwrap();
    assert_ne!(before.as_bytes(), after.as_bytes());
    drop(workspace);

    let reopened = Workspace::open_as(&project, actor, session).unwrap();
    assert_eq!(
        reopened.document.layer(1).unwrap().dissolve_seed,
        0x8765_4321
    );
    assert_eq!(
        render_document(&reopened.document, None)
            .unwrap()
            .as_bytes(),
        after.as_bytes()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dissolve_transfer_requires_v6_and_preserves_the_seed() {
    let document = fixture().document;
    let transfer = LayerTransfer::from_document(&document, 1).unwrap();
    assert_eq!(transfer.version, 6);
    assert_eq!(transfer.layer.blend_mode, BlendMode::Dissolve);
    assert_eq!(transfer.layer.dissolve_seed, 0x1234_5678);
    assert_eq!(
        LayerTransfer::from_json(&transfer.to_json().unwrap()).unwrap(),
        transfer
    );
}

#[test]
fn old_documents_default_to_a_zero_seed() {
    let json = r#"{
        "name":"Legacy","width":2,"height":2,
        "layers":[{"id":1,"blend_mode":"normal","kind":{
            "type":"rectangle","width":2,"height":2,
            "color":[255,255,255,255],"corner_radius":0.0
        }}]
    }"#;
    let document: Document = serde_json::from_str(json).unwrap();
    assert_eq!(document.layers[0].dissolve_seed, 0);
}
