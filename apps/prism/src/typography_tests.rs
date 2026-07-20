use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    Command, Document, LayerKind, TextAlignment, TextEffects, TextTypography, Workspace,
    render_document, render_layer_base_scaled_with_font,
};

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-typography-{label}-{stamp}"))
}

fn test_actor() -> spectrum_revisions::Actor {
    spectrum_revisions::Actor {
        id: "person:typography-test".into(),
        display_name: "Typography test".into(),
        kind: spectrum_revisions::ActorKind::Human,
    }
}

#[test]
fn old_text_json_migrates_without_losing_guides_or_text() {
    let mut value = serde_json::to_value(Document::new("Legacy", 320, 200)).unwrap();
    value["version"] = serde_json::json!(1);
    value["layers"] = serde_json::json!([{
        "id": 1,
        "name": "Legacy text",
        "visible": true,
        "locked": false,
        "opacity": 1.0,
        "blend_mode": "normal",
        "transform": {},
        "adjustments": {},
        "mask": {},
        "stroke": {},
        "clip_to_below": false,
        "kind": {"type": "text", "text": "Legacy", "font_size": 48.0, "color": [255,255,255,255]}
    }]);
    value.as_object_mut().unwrap().remove("font_assets");
    value.as_object_mut().unwrap().remove("next_font_id");
    let mut document: Document = serde_json::from_value(value).unwrap();
    document.migrate().unwrap();
    let LayerKind::Text { typography, .. } = &document.layers[0].kind else {
        panic!("legacy layer should remain text");
    };
    assert_eq!(typography, &TextTypography::default());
    assert!(document.font_assets.is_empty());
    assert!(document.snapping_enabled);
}

#[test]
fn imported_font_and_typography_round_trip_inside_durable_project() {
    let directory = test_directory("portable-font");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let project = directory.join("portable.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Portable typography", 640, 360),
        &project,
        test_actor(),
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: source.clone(),
        })
        .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: source.clone(),
        })
        .unwrap();
    assert_eq!(workspace.document.font_assets.len(), 1);
    let font_id = workspace.document.font_assets[0].id;
    workspace
        .execute(Command::AddText {
            text: "Portable\nfont".into(),
            name: None,
            font_size: 56.0,
            color: [240, 220, 180, 255],
            x: 30.0,
            y: 40.0,
        })
        .unwrap();
    let layer_id = workspace.document.selected.unwrap();
    let typography = TextTypography {
        font_id: Some(font_id),
        alignment: TextAlignment::Right,
        line_height: 1.4,
        tracking: 3.0,
        box_width: Some(260.0),
        ..Default::default()
    };
    workspace
        .execute(Command::SetTextTypography {
            id: layer_id,
            typography: typography.clone(),
        })
        .unwrap();
    workspace.save(None).unwrap();
    drop(workspace);
    fs::remove_file(&source).unwrap();

    let loaded = Workspace::load_read_only(&project).unwrap();
    assert_eq!(loaded.font_assets.len(), 1);
    assert!(loaded.font_assets[0].path.exists());
    assert_ne!(loaded.font_assets[0].path, source);
    assert!(loaded.font_assets[0].original_path.is_none());
    assert_eq!(
        loaded.font_assets[0].bytes().unwrap(),
        epaint_default_fonts::HACK_REGULAR
    );
    let LayerKind::Text {
        typography: loaded_typography,
        ..
    } = &loaded.layer(layer_id).unwrap().kind
    else {
        panic!("text layer should survive durable replay");
    };
    assert_eq!(loaded_typography, &typography);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn malformed_font_import_is_rejected_without_allocating_an_asset() {
    let directory = test_directory("malformed-font");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("broken.ttf");
    fs::write(&source, b"not an OpenType font").unwrap();
    let mut workspace = Workspace::new(Document::new("Type", 320, 200), None);

    assert!(
        workspace
            .execute(Command::ImportFont { path: source })
            .is_err()
    );
    assert!(workspace.document.font_assets.is_empty());
    assert_eq!(workspace.document.next_font_id, 1);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn migration_recovers_a_missing_font_reference_to_the_bundled_face() {
    let mut document = Document::new("Recovery", 320, 200);
    document.layers.push(crate::Layer {
        id: 1,
        kind: LayerKind::Text {
            text: "Fallback".into(),
            font_size: 48.0,
            color: [255; 4],
            typography: TextTypography {
                font_id: Some(404),
                ..Default::default()
            },
        },
        ..Default::default()
    });
    document.migrate().unwrap();

    let LayerKind::Text { typography, .. } = &document.layers[0].kind else {
        panic!("layer should remain text");
    };
    assert_eq!(typography.font_id, None);
}

#[test]
fn typography_command_rejects_unknown_font_ids_and_sanitizes_metrics() {
    let mut workspace = Workspace::new(Document::new("Type", 320, 200), None);
    workspace
        .execute(Command::AddText {
            text: "Text".into(),
            name: None,
            font_size: 48.0,
            color: [255; 4],
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    let id = workspace.document.selected.unwrap();
    let unknown = TextTypography {
        font_id: Some(99),
        ..Default::default()
    };
    assert!(
        workspace
            .execute(Command::SetTextTypography {
                id,
                typography: unknown,
            })
            .is_err()
    );
    workspace
        .execute(Command::SetTextTypography {
            id,
            typography: TextTypography {
                line_height: 99.0,
                tracking: -999.0,
                box_width: Some(-20.0),
                ..Default::default()
            },
        })
        .unwrap();
    let LayerKind::Text { typography, .. } = &workspace.document.layer(id).unwrap().kind else {
        panic!("layer should remain text");
    };
    assert_eq!(typography.line_height, 4.0);
    assert_eq!(typography.tracking, -100.0);
    assert_eq!(typography.box_width, Some(1.0));
}

#[test]
fn imported_font_id_changes_shared_preview_and_export_pixels() {
    let directory = test_directory("rendered-font");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let mut workspace = Workspace::new(Document::new("Rendered font", 520, 260), None);
    workspace
        .execute(Command::ImportFont {
            path: source.clone(),
        })
        .unwrap();
    let font_id = workspace.document.font_assets[0].id;
    workspace
        .execute(Command::AddText {
            text: "Wide WWW\nthen iii".into(),
            name: None,
            font_size: 52.0,
            color: [238, 214, 151, 255],
            x: 24.0,
            y: 20.0,
        })
        .unwrap();
    let layer_id = workspace.document.selected.unwrap();
    workspace
        .execute(Command::SetTextTypography {
            id: layer_id,
            typography: TextTypography {
                font_id: Some(font_id),
                alignment: TextAlignment::Center,
                line_height: 1.45,
                tracking: 2.0,
                box_width: Some(360.0),
                effects: TextEffects {
                    outline_width: 2.0,
                    shadow_offset_x: 5.0,
                    shadow_offset_y: 7.0,
                    shadow_color: [23, 31, 47, 160],
                    ..Default::default()
                },
            },
        })
        .unwrap();

    let imported_layer = workspace.document.layer(layer_id).unwrap();
    let imported_font = workspace.document.font_for_layer(imported_layer).unwrap();
    let imported_preview =
        render_layer_base_scaled_with_font(imported_layer, None, [1.0; 2], Some(imported_font))
            .unwrap();
    let imported_export = render_document(&workspace.document, None).unwrap();

    let mut fallback = workspace.document.clone();
    let LayerKind::Text { typography, .. } = &mut fallback.layer_mut(layer_id).unwrap().kind else {
        panic!("test layer should remain text");
    };
    typography.font_id = None;
    let fallback_layer = fallback.layer(layer_id).unwrap();
    let fallback_preview =
        render_layer_base_scaled_with_font(fallback_layer, None, [1.0; 2], None).unwrap();
    let fallback_export = render_document(&fallback, None).unwrap();

    assert_ne!(imported_preview.to_rgba8(), fallback_preview.to_rgba8());
    assert_ne!(imported_export.to_rgba8(), fallback_export.to_rgba8());
    fs::remove_dir_all(directory).unwrap();
}
