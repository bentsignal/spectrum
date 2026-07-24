use crate::*;

fn styled_shape_document() -> Document {
    let mut document = Document::new("Effects", 80, 50);
    document.background = [0, 0, 0, 0];
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            x: 10.0,
            y: 10.0,
            ..Transform::default()
        },
        style: LayerStyle {
            drop_shadow: Some(DropShadow {
                color: [0, 0, 0, 180],
                offset_x: 20.0,
                offset_y: 0.0,
                blur_radius: 0.0,
            }),
        },
        shape_fill: Some(ShapeFill::Gradient(ShapeGradient {
            angle: 0.0,
            stops: vec![
                GradientStop::new(0.0, [255, 0, 0, 255]),
                GradientStop::new(1.0, [0, 0, 255, 255]),
            ],
            ..ShapeGradient::default()
        })),
        kind: LayerKind::Rectangle {
            width: 20,
            height: 20,
            color: [255, 255, 255, 255],
            corner_radius: 0.0,
        },
        ..Layer::default()
    });
    document.selected = Some(1);
    document.next_id = 2;
    document
}

#[test]
fn version_two_documents_migrate_with_compatible_effect_defaults() {
    let mut value = serde_json::to_value(Document::new("Legacy", 64, 48)).unwrap();
    value["version"] = serde_json::json!(2);
    let layer = serde_json::to_value(Layer::default()).unwrap();
    value["layers"] = serde_json::json!([layer]);
    value["layers"][0].as_object_mut().unwrap().remove("style");
    value["layers"][0]
        .as_object_mut()
        .unwrap()
        .remove("shape_fill");
    let mut document: Document = serde_json::from_value(value).unwrap();
    document.migrate().unwrap();
    assert_eq!(document.version, PRISM_VERSION);
    assert!(document.layers[0].style.is_empty());
    assert!(document.layers[0].shape_fill.is_none());
}

#[test]
fn style_and_fill_commands_validate_layer_kind_and_undo_separately() {
    let mut workspace = Workspace::new(Document::new("Commands", 100, 100), None);
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 40,
            height: 30,
            color: [40, 80, 120, 255],
            corner_radius: 0.0,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetLayerStyle {
            id: 1,
            style: LayerStyle {
                drop_shadow: Some(DropShadow::default()),
            },
        })
        .unwrap();
    workspace
        .execute(Command::SetShapeFill {
            id: 1,
            fill: Some(ShapeFill::Gradient(ShapeGradient::default())),
        })
        .unwrap();
    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.layer(1).unwrap().shape_fill.is_none());
    assert!(
        workspace
            .document
            .layer(1)
            .unwrap()
            .style
            .drop_shadow
            .is_some()
    );

    workspace
        .execute(Command::AddText {
            text: "No gradient".into(),
            name: None,
            font_size: 24.0,
            color: [255; 4],
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    assert!(
        workspace
            .execute(Command::SetShapeFill {
                id: 2,
                fill: Some(ShapeFill::Gradient(ShapeGradient::default())),
            })
            .is_err()
    );
}

#[test]
fn previewed_effect_changes_commit_as_one_undoable_revision() {
    let mut workspace = Workspace::new(styled_shape_document(), None);
    let original = workspace.document.layer(1).unwrap().style.clone();
    workspace.begin_interaction();
    for offset_x in [24.0, 28.0, 32.0] {
        let mut style = original.clone();
        style.drop_shadow.as_mut().unwrap().offset_x = offset_x;
        workspace
            .preview(Command::SetLayerStyle { id: 1, style })
            .unwrap();
    }
    assert!(workspace.commit_interaction().unwrap());
    assert_eq!(
        workspace
            .document
            .layer(1)
            .unwrap()
            .style
            .drop_shadow
            .unwrap()
            .offset_x,
        32.0
    );
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(workspace.document.layer(1).unwrap().style, original);
}

#[test]
fn gradient_and_shadow_match_full_export_and_exact_region_preview() {
    let document = styled_shape_document();
    let full = render_document_scaled(&document, 1.0).unwrap().to_rgba8();
    let (preview, stats) = render_document_region_scaled_with_stats(
        &document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: document.width,
            height: document.height,
        },
    )
    .unwrap();
    assert_eq!(preview.to_rgba8(), full);
    assert!(full.get_pixel(11, 20)[0] > full.get_pixel(28, 20)[0]);
    assert!(full.get_pixel(28, 20)[2] > full.get_pixel(11, 20)[2]);
    assert!(full.get_pixel(35, 20)[3] > 0, "shadow extends beyond shape");
    assert!(stats.shadow_samples <= stats.output_pixels * 13);
}

#[test]
fn rotated_masked_clipped_shadow_matches_full_export() {
    let mut document = styled_shape_document();
    document.width = 120;
    document.height = 90;
    let mut styled = document.layers.remove(0);
    styled.transform.x = 34.0;
    styled.transform.y = 20.0;
    styled.transform.rotation = 31.0;
    styled.mask = LayerMask {
        enabled: true,
        invert: false,
        x: 0.15,
        y: 0.2,
        width: 0.45,
        height: 0.55,
    };
    styled.blend_mode = BlendMode::Screen;
    styled.clip_to_below = true;
    styled.adjustments = spectrum_imaging::Adjustments {
        exposure: 0.2,
        noise_reduction: 12.0,
        sharpening: 9.0,
        straighten: 2.5,
        ..Default::default()
    };
    document.layers.push(Layer {
        id: 2,
        transform: Transform {
            x: 8.0,
            y: 8.0,
            ..Transform::default()
        },
        kind: LayerKind::Rectangle {
            width: 78,
            height: 62,
            color: [40, 80, 120, 210],
            corner_radius: 6.0,
        },
        ..Layer::default()
    });
    document.layers.push(styled);
    document.next_id = 3;

    let full = render_document_scaled(&document, 1.0).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 12,
        y: 8,
        width: 96,
        height: 74,
    };
    let (preview, stats) =
        render_document_region_scaled_with_stats(&document, 1.0, region).unwrap();
    let preview = preview.to_rgba8();
    let oracle = image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
        .to_image();
    assert_eq!(preview, oracle);
    assert!(stats.adjusted_staging_pixels > 0);
    assert!(stats.shadow_alpha_tile_pixels > 0);
    assert_eq!(
        stats.shadow_alpha_tile_bytes,
        stats.shadow_alpha_tile_pixels
    );
    assert!(stats.max_shadow_alpha_tile_pixels <= 4_096 * 4_096);
    assert_eq!(
        stats.shadow_source_samples, stats.shadow_alpha_tile_pixels,
        "adjusted shadows must not fall back to direct source-alpha sampling"
    );
}

#[test]
fn huge_gradient_shadow_preview_stays_viewport_bounded() {
    let mut document = styled_shape_document();
    document.width = MAX_CANVAS_DIMENSION;
    document.height = MAX_CANVAS_DIMENSION;
    document.layers[0].kind = LayerKind::Rectangle {
        width: MAX_CANVAS_DIMENSION,
        height: MAX_CANVAS_DIMENSION,
        color: [255; 4],
        corner_radius: 0.0,
    };
    document.layers[0]
        .style
        .drop_shadow
        .as_mut()
        .unwrap()
        .blur_radius = 24.0;
    let (_, stats) = render_document_region_scaled_with_stats(
        &document,
        8.0,
        RenderRegion {
            x: 20_000,
            y: 20_000,
            width: 320,
            height: 180,
        },
    )
    .unwrap();
    assert_eq!(stats.source_staging_pixels, 0);
    assert_eq!(stats.transformed_surface_pixels, 0);
    assert!(stats.shadow_samples <= stats.output_pixels * 13);
    assert!(stats.shadow_alpha_tile_pixels > 0);
    assert_eq!(
        stats.shadow_alpha_tile_bytes,
        stats.shadow_alpha_tile_pixels
    );
    assert!(stats.max_shadow_alpha_tile_pixels <= 4_096 * 4_096);
    assert_eq!(
        stats.max_shadow_alpha_tile_bytes,
        stats.max_shadow_alpha_tile_pixels
    );
}

#[test]
fn durable_effect_edits_use_the_version_three_operation_envelope() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("prism-effects-{stamp}"));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("effects.prism");
    let actor = spectrum_revisions::Actor {
        id: "person:effect-test".into(),
        display_name: "Effect tester".into(),
        kind: spectrum_revisions::ActorKind::Human,
    };
    let mut workspace = Workspace::create_durable(
        Document::new("Effects", 80, 60),
        &project,
        actor,
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 20,
            height: 20,
            color: [255; 4],
            corner_radius: 0.0,
            x: 5.0,
            y: 5.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetLayerStyle {
            id: 1,
            style: LayerStyle {
                drop_shadow: Some(DropShadow::default()),
            },
        })
        .unwrap();
    workspace
        .execute(Command::SetShapeFill {
            id: 1,
            fill: Some(ShapeFill::Gradient(ShapeGradient::default())),
        })
        .unwrap();
    workspace.save(None).unwrap();
    drop(workspace);
    let connection = rusqlite::Connection::open(&project).unwrap();
    let effect_versions: u32 = connection
        .query_row(
            "SELECT count(*) FROM operation_payloads WHERE version = 3",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(effect_versions, 2);
    let effect_snapshots: u32 = connection
        .query_row(
            "SELECT count(*) FROM snapshots WHERE version = 4",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(effect_snapshots, 1);
    drop(connection);

    let mut reopened = Workspace::open(&project).unwrap();
    assert!(reopened.document.layer(1).unwrap().shape_fill.is_some());
    assert!(
        reopened
            .document
            .layer(1)
            .unwrap()
            .style
            .drop_shadow
            .is_some()
    );
    reopened.execute(Command::Undo).unwrap();
    assert!(reopened.document.layer(1).unwrap().shape_fill.is_none());
    assert!(
        reopened
            .document
            .layer(1)
            .unwrap()
            .style
            .drop_shadow
            .is_some()
    );
    reopened.execute(Command::Undo).unwrap();
    assert!(reopened.document.layer(1).unwrap().style.is_empty());
    reopened.execute(Command::Redo).unwrap();
    reopened.execute(Command::Redo).unwrap();
    assert!(reopened.document.layer(1).unwrap().shape_fill.is_some());
    assert!(
        reopened
            .document
            .layer(1)
            .unwrap()
            .style
            .drop_shadow
            .is_some()
    );
    drop(reopened);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_styled_layer_insert_uses_the_version_three_envelope() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("prism-styled-transfer-{stamp}"));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("styled-transfer.prism");
    let transfer = LayerTransfer::from_document(&styled_shape_document(), 1).unwrap();
    let mut workspace = Workspace::create_durable(
        Document::new("Styled transfer", 100, 80),
        &project,
        spectrum_revisions::Actor {
            id: "person:styled-transfer".into(),
            display_name: "Styled transfer tester".into(),
            kind: spectrum_revisions::ActorKind::Human,
        },
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::InsertLayer {
            transfer: Box::new(transfer),
            index: None,
        })
        .unwrap();
    workspace.save(None).unwrap();
    drop(workspace);

    let connection = rusqlite::Connection::open(&project).unwrap();
    let (version, bytes): (u32, Vec<u8>) = connection
        .query_row(
            "SELECT version, bytes FROM operation_payloads LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(version, 3);
    let commands: Vec<Command> = serde_json::from_slice(&bytes).unwrap();
    let Command::InsertLayer { transfer, .. } = &commands[0] else {
        panic!("durable command should remain a layer insert");
    };
    assert_eq!(
        transfer.version, 2,
        "unmasked styled transfers remain readable by the v2 transfer schema"
    );
    drop(connection);

    let reopened = Workspace::load_read_only(&project).unwrap();
    assert!(reopened.layers[0].style.drop_shadow.is_some());
    assert!(reopened.layers[0].shape_fill.is_some());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn legacy_transfer_envelopes_cannot_smuggle_new_style_fields() {
    let mut transfer = LayerTransfer::from_document(&styled_shape_document(), 1).unwrap();
    transfer.version = 1;
    assert!(LayerTransfer::from_json(&serde_json::to_string(&transfer).unwrap()).is_err());
    transfer.layer.style = LayerStyle::default();
    transfer.layer.shape_fill = None;
    assert!(LayerTransfer::from_json(&serde_json::to_string(&transfer).unwrap()).is_ok());
}
