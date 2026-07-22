use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    Command, Document, FontAsset, FontEmbeddingPermission, FontSourceSnapshot, LayerKind,
    TextAlignment, TextEffects, TextTypography, Workspace, analyze_all_font_usage,
    analyze_font_usage, font_usage, render_document, render_layer_base_scaled_with_font,
    save_document,
};

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!("prism-typography-{label}-{stamp}"))
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
            source_name: None,
        })
        .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: source.clone(),
            source_name: None,
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
        loaded.font_assets[0].content_hash,
        spectrum_revisions::AssetId::for_bytes(epaint_default_fonts::HACK_REGULAR).to_string()
    );
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
            .execute(Command::ImportFont {
                path: source,
                source_name: None,
            })
            .is_err()
    );
    assert!(workspace.document.font_assets.is_empty());
    assert_eq!(workspace.document.next_font_id, 1);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn immutable_font_source_rejects_tampering_truncation_and_unmaterialized_paths() {
    let directory = test_directory("immutable-source");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let asset = FontAsset::import(1, &source).unwrap();
    let snapshot = asset.source_snapshot().unwrap();
    assert_eq!(snapshot.bytes(), epaint_default_fonts::HACK_REGULAR);
    assert_eq!(snapshot.content_hash(), asset.content_hash);

    let mut wrong_identity = asset.clone();
    wrong_identity.content_hash = "0".repeat(64);
    assert!(
        wrong_identity
            .source_snapshot()
            .unwrap_err()
            .to_string()
            .contains("content identity")
    );

    let mut escaped = asset.clone();
    escaped.path = PathBuf::from("../outside-project-font.ttf");
    assert!(
        escaped
            .source_snapshot()
            .unwrap_err()
            .to_string()
            .contains("not materialized safely")
    );

    fs::write(&source, &epaint_default_fonts::HACK_REGULAR[..128]).unwrap();
    assert!(asset.source_snapshot().is_err());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn legacy_and_initial_durable_saves_reject_tampered_font_bytes() {
    let directory = test_directory("tampered-save");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let mut document = Document::new("Tampered save", 320, 200);
    document
        .font_assets
        .push(FontAsset::import(1, &source).unwrap());
    fs::write(
        &source,
        vec![0x4f; epaint_default_fonts::HACK_REGULAR.len()],
    )
    .unwrap();
    let legacy_project = directory.join("tampered-legacy.prism");
    let durable_project = directory.join("tampered-durable.prism");

    let legacy_error = save_document(&document, &legacy_project).unwrap_err();
    let durable_error = Workspace::create_durable(
        document,
        &durable_project,
        test_actor(),
        spectrum_revisions::SessionId::new(),
    )
    .err()
    .expect("tampered initial snapshot should fail");

    assert!(legacy_error.to_string().contains("content identity"));
    assert!(durable_error.to_string().contains("content identity"));
    assert!(!legacy_project.exists());
    assert!(!durable_project.exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn font_source_read_is_bounded_before_allocating_file_contents() {
    let directory = test_directory("oversized-source");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("oversized.ttf");
    fs::File::create(&source)
        .unwrap()
        .set_len(crate::font_source::MAX_EMBEDDED_FONT_BYTES as u64 + 1)
        .unwrap();
    assert!(
        FontSourceSnapshot::read(&source)
            .unwrap_err()
            .to_string()
            .contains("32 MiB")
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn source_snapshot_accepts_static_variable_and_cff_open_type_containers() {
    let fixtures = fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/spectrum-fonts/tests/fonts"),
    )
    .unwrap();
    for name in [
        "noto-sans-static-source.ttf",
        "noto-sans-variable-rejected.ttf",
        "noto-sans-cff-rejected.otf",
    ] {
        let snapshot = FontSourceSnapshot::read(&fixtures.join(name)).unwrap();
        assert!(!snapshot.is_empty(), "fixture {name} should be readable");
        assert_eq!(snapshot.content_hash().len(), 64);
    }
}

#[test]
fn font_source_accepts_license_restrictions_but_rejects_bitmap_only_embedding() {
    let directory = test_directory("embedding-restrictions");
    fs::create_dir_all(&directory).unwrap();
    let preview = directory.join("preview.ttf");
    let mut preview_bytes = epaint_default_fonts::HACK_REGULAR.to_vec();
    set_fs_type(&mut preview_bytes, 0x0104);
    fs::write(&preview, &preview_bytes).unwrap();
    let snapshot = FontSourceSnapshot::read(&preview).unwrap();
    assert_eq!(
        snapshot.embedding_permission(),
        FontEmbeddingPermission::PreviewAndPrint
    );
    assert!(!snapshot.subset_allowed());

    let restricted = directory.join("restricted.ttf");
    let mut restricted_bytes = epaint_default_fonts::HACK_REGULAR.to_vec();
    set_fs_type(&mut restricted_bytes, 0x0002);
    fs::write(&restricted, restricted_bytes).unwrap();
    let snapshot = FontSourceSnapshot::read(&restricted).unwrap();
    assert_eq!(
        snapshot.embedding_permission(),
        FontEmbeddingPermission::Restricted
    );
    assert!(!snapshot.subset_allowed());

    let bitmap_only = directory.join("bitmap-only.ttf");
    let mut bitmap_only_bytes = epaint_default_fonts::HACK_REGULAR.to_vec();
    set_fs_type(&mut bitmap_only_bytes, 0x0208);
    fs::write(&bitmap_only, bitmap_only_bytes).unwrap();
    assert!(FontSourceSnapshot::read(&bitmap_only).is_err());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn preview_print_font_import_warns_and_reloads_without_losing_bytes_or_policy() {
    assert_font_permission_round_trip(
        "preview-print-round-trip",
        "Preview-Print.ttf",
        0x0104,
        FontEmbeddingPermission::PreviewAndPrint,
        "preview/print",
    );
}

#[test]
fn restricted_font_import_warns_renders_and_disables_subsetting_after_reload() {
    assert_font_permission_round_trip(
        "restricted-round-trip",
        "Restricted.ttf",
        0x0002,
        FontEmbeddingPermission::Restricted,
        "restricted embedding",
    );
}

#[test]
fn legacy_editable_font_metadata_is_hydrated_from_immutable_source_bytes() {
    let directory = test_directory("legacy-editable-permission");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Editable.ttf");
    let mut bytes = epaint_default_fonts::HACK_REGULAR.to_vec();
    set_fs_type(&mut bytes, 0x0008);
    fs::write(&source, &bytes).unwrap();
    let current = FontAsset::import(1, &source).unwrap();
    assert_eq!(
        current.embedding_permission,
        FontEmbeddingPermission::Editable
    );
    let mut legacy = serde_json::to_value(&current).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("embedding_permission");
    let mut loaded: FontAsset = serde_json::from_value(legacy).unwrap();
    assert_eq!(
        loaded.embedding_permission,
        FontEmbeddingPermission::LegacyUnknown
    );

    loaded.hydrate_legacy_embedding_permission().unwrap();

    assert_eq!(
        loaded.embedding_permission,
        FontEmbeddingPermission::Editable
    );
    assert_eq!(loaded.bytes().unwrap(), bytes);
    fs::remove_dir_all(directory).unwrap();
}

fn assert_font_permission_round_trip(
    label: &str,
    source_name: &str,
    fs_type: u16,
    expected_permission: FontEmbeddingPermission,
    warning_fragment: &str,
) {
    let directory = test_directory(label);
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join(source_name);
    let mut source_bytes = epaint_default_fonts::HACK_REGULAR.to_vec();
    set_fs_type(&mut source_bytes, fs_type);
    fs::write(&source, &source_bytes).unwrap();
    let project = directory.join("font-policy.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Preview print", 640, 360),
        &project,
        test_actor(),
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();

    let output = workspace
        .execute(Command::ImportFont {
            path: source.clone(),
            source_name: None,
        })
        .unwrap();
    assert_eq!(output.warnings.len(), 1);
    assert!(output.warnings[0].contains(warning_fragment));
    let font_id = workspace.document.font_assets[0].id;
    assert_eq!(
        workspace.document.font_assets[0].embedding_permission,
        expected_permission
    );
    assert_eq!(workspace.document.font_assets[0].source_name, source_name);
    assert!(!workspace.document.font_assets[0].subset_allowed);
    workspace
        .execute(Command::AddText {
            text: "Editable preview font".into(),
            name: None,
            font_size: 42.0,
            color: [255; 4],
            x: 30.0,
            y: 40.0,
        })
        .unwrap();
    let layer_id = workspace.document.selected.unwrap();
    workspace
        .execute(Command::SetTextTypography {
            id: layer_id,
            typography: TextTypography {
                font_id: Some(font_id),
                ..Default::default()
            },
        })
        .unwrap();
    let subset_plan = crate::plan_font_subset(&workspace.document, font_id).unwrap();
    assert!(!subset_plan.analysis.embedding_metadata_allows_subsetting);
    assert!(
        subset_plan
            .candidate_blockers
            .iter()
            .any(|blocker| blocker.contains("forbids technical subsetting"))
    );
    assert!(render_document(&workspace.document, None).is_ok());
    drop(workspace);
    fs::remove_file(&source).unwrap();

    let loaded = Workspace::load_read_only(&project).unwrap();
    assert_eq!(
        loaded.font_assets[0].embedding_permission,
        expected_permission
    );
    assert_eq!(loaded.font_assets[0].source_name, source_name);
    assert!(!loaded.font_assets[0].subset_allowed);
    assert_eq!(loaded.font_assets[0].bytes().unwrap(), source_bytes);
    assert!(render_document(&loaded, None).is_ok());
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn font_source_rejects_symlink_entry_points() {
    use std::os::unix::fs::symlink;

    let directory = test_directory("symlink-source");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    let link = directory.join("linked.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    symlink(&source, &link).unwrap();
    assert!(
        FontSourceSnapshot::read(&link)
            .unwrap_err()
            .to_string()
            .contains("securely open")
    );
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn font_source_rejects_symlinked_ancestor_directories() {
    use std::os::unix::fs::symlink;

    let directory = test_directory("symlink-ancestor");
    let real_directory = directory.join("real");
    fs::create_dir_all(&real_directory).unwrap();
    let source = real_directory.join("Hack-Regular.ttf");
    let linked_directory = directory.join("linked");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    symlink(&real_directory, &linked_directory).unwrap();
    assert!(
        FontSourceSnapshot::read(&linked_directory.join("Hack-Regular.ttf"))
            .unwrap_err()
            .to_string()
            .contains("securely open")
    );
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_font_source_binds_a_regular_file_handle_identity() {
    let directory = test_directory("windows-file-identity");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let snapshot = FontSourceSnapshot::read(&source).unwrap();
    assert_eq!(snapshot.bytes(), epaint_default_fonts::HACK_REGULAR);
    assert!(snapshot.canonical_path().is_absolute());
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_font_source_rejects_reparse_point_ancestors_when_links_are_available() {
    use std::os::windows::fs::symlink_dir;

    let directory = test_directory("windows-reparse-ancestor");
    let real_directory = directory.join("real");
    fs::create_dir_all(&real_directory).unwrap();
    let source = real_directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let linked_directory = directory.join("linked");
    if symlink_dir(&real_directory, &linked_directory).is_ok() {
        assert!(
            FontSourceSnapshot::read(&linked_directory.join("Hack-Regular.ttf"))
                .unwrap_err()
                .to_string()
                .contains("reparse-point")
        );
    }
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_font_source_rejects_an_ancestor_junction_redirect() {
    let directory = test_directory("windows-junction-ancestor");
    let real_directory = directory.join("real");
    fs::create_dir_all(&real_directory).unwrap();
    let source = real_directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let junction = directory.join("junction");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&real_directory)
        .status()
        .unwrap();
    assert!(status.success(), "test junction should be creatable");

    let error = FontSourceSnapshot::read(&junction.join("Hack-Regular.ttf")).unwrap_err();

    assert!(error.to_string().contains("reparse-point"));
    fs::remove_dir(&junction).unwrap();
    fs::remove_dir_all(directory).unwrap();
}

fn set_fs_type(bytes: &mut [u8], fs_type: u16) {
    let table_count = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    for index in 0..table_count {
        let record = 12 + index * 16;
        if &bytes[record..record + 4] != b"OS/2" {
            continue;
        }
        let offset = u32::from_be_bytes([
            bytes[record + 8],
            bytes[record + 9],
            bytes[record + 10],
            bytes[record + 11],
        ]) as usize;
        bytes[offset + 8..offset + 10].copy_from_slice(&fs_type.to_be_bytes());
        return;
    }
    panic!("test font has no OS/2 table");
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
            source_name: None,
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

#[test]
fn font_usage_analysis_is_sorted_deduplicated_and_non_mutating() {
    let directory = test_directory("font-usage");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let mut workspace = Workspace::new(Document::new("Font usage", 520, 260), None);
    workspace
        .execute(Command::ImportFont {
            path: source.clone(),
            source_name: None,
        })
        .unwrap();
    let font_id = workspace.document.font_assets[0].id;
    for text in ["BA\u{fe0f}\nA".to_owned(), format!(" A{}", '\u{10ffff}')] {
        workspace
            .execute(Command::AddText {
                text,
                name: None,
                font_size: 32.0,
                color: [255; 4],
                x: 0.0,
                y: 0.0,
            })
            .unwrap();
        let id = workspace.document.selected.unwrap();
        workspace
            .execute(Command::SetTextTypography {
                id,
                typography: TextTypography {
                    font_id: Some(font_id),
                    ..Default::default()
                },
            })
            .unwrap();
    }
    let before = workspace.document.clone();
    let usage = font_usage(&workspace.document, font_id).unwrap();
    assert_eq!(usage.layer_ids, vec![1, 2]);
    assert_eq!(usage.codepoints, vec![32, 65, 66, 0x10ffff]);
    assert_eq!(
        usage.variation_sequences,
        vec![crate::UnicodeVariationSequence {
            base_codepoint: 65,
            selector_codepoint: 0xfe0f,
        }]
    );
    assert!(usage.unpaired_variation_selectors.is_empty());

    let analysis = analyze_font_usage(&workspace.document, font_id).unwrap();
    assert_eq!(analysis.usage, usage);
    assert_eq!(
        analysis.source_bytes,
        epaint_default_fonts::HACK_REGULAR.len() as u64
    );
    assert_eq!(analysis.missing_codepoints, vec![0x10ffff]);
    assert_eq!(
        analysis.missing_variation_sequences,
        usage.variation_sequences
    );
    assert_eq!(analysis.source_name, "Hack-Regular.ttf");
    let canonical_source = fs::canonicalize(&source).unwrap();
    assert_eq!(
        analysis.original_path.as_deref(),
        Some(canonical_source.as_path())
    );
    assert_eq!(
        analyze_all_font_usage(&workspace.document).unwrap(),
        vec![analysis]
    );
    assert_eq!(workspace.document, before);

    fs::remove_dir_all(directory).unwrap();
}
