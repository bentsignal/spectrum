use std::fs;

use spectrum_revisions::{Actor, ActorKind, CollaborationMode, RevisionStore, SessionId};

use crate::{Command, Document, LayerKind, TextShaping, TextTypography, Workspace};

use super::*;

const LOCL_FONT: &[u8] =
    include_bytes!("../../../crates/spectrum-fonts/tests/fonts/noto-sans-locl-source.ttf");

fn actor() -> Actor {
    Actor {
        id: "test:optimized-copy-shaping".into(),
        display_name: "Optimized Copy Shaping Test".into(),
        kind: ActorKind::Human,
    }
}

#[test]
fn historical_shaped_locl_state_survives_after_its_layer_is_removed_at_tip() {
    let directory = fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!(
            "prism-optimized-historical-locl-{}",
            SessionId::new()
        ));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.prism");
    let output = directory.join("optimized.prism");
    let font_path = directory.join("NotoSansItalicLocl.ttf");
    fs::write(&font_path, LOCL_FONT).unwrap();
    let mut workspace = Workspace::create_durable(
        Document::new("Historical locl", 320, 200),
        &source,
        actor(),
        SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: font_path,
            source_name: None,
        })
        .unwrap();
    let font_id = workspace.document.font_assets[0].id;
    let source_hash = workspace.document.font_assets[0].content_hash.clone();
    workspace
        .execute(Command::AddText {
            text: "Aé бгдпт".into(),
            name: Some("Serbian historical state".into()),
            font_size: 48.0,
            color: [255; 4],
            x: 8.0,
            y: 12.0,
            shaping: TextShaping::harfbuzz_v1(Some("sr")).unwrap(),
        })
        .unwrap();
    let layer_id = workspace.document.selected.unwrap();
    workspace
        .execute(Command::SetTextTypography {
            id: layer_id,
            typography: TextTypography {
                font_id: Some(font_id),
                shaping: TextShaping::harfbuzz_v1(Some("sr")).unwrap(),
                ..Default::default()
            },
        })
        .unwrap();
    let historical_index = workspace.history().unwrap().unwrap().revisions.len() - 1;
    workspace
        .execute(Command::RemoveLayer { id: layer_id })
        .unwrap();
    assert!(workspace.document.layers.is_empty());
    drop(workspace);
    for index in 0..128 {
        Workspace::start_collaboration(
            &source,
            None,
            Actor {
                id: format!("test:optimized-copy-follower-{index}"),
                display_name: format!("Follower {index}"),
                kind: ActorKind::Agent,
            },
            CollaborationMode::Separate,
        )
        .unwrap();
    }

    create_optimized_font_copy(&source, &output).unwrap();
    let destination = RevisionStore::open_read_only(&output).unwrap();
    let (revisions, _) = linear_history(&destination, &output).unwrap();
    let revision_ids = revisions
        .iter()
        .map(|revision| revision.id)
        .collect::<Vec<_>>();
    drop(destination);

    let session = SessionId::new();
    let mut destination = Workspace::open_as(&output, actor(), session).unwrap();
    destination
        .move_to_revision(revision_ids[historical_index])
        .unwrap();
    let LayerKind::Text {
        text, typography, ..
    } = &destination.document.layers[0].kind
    else {
        panic!("historical destination state must retain its text layer");
    };
    assert_eq!(text, "Aé бгдпт");
    assert_eq!(typography.shaping.resolved_language(), "sr");
    assert_ne!(
        destination.document.font_assets[0].content_hash,
        source_hash
    );
    destination
        .move_to_revision(*revision_ids.last().unwrap())
        .unwrap();
    assert!(destination.document.layers.is_empty());
    drop(destination);
    let reopened = Workspace::open_session(&output, session).unwrap();
    assert!(reopened.document.layers.is_empty());

    fs::remove_dir_all(directory).unwrap();
}
