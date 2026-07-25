use super::*;

fn document_with_authoring_only_clone_marker() -> Document {
    let paint_stroke = BrushStroke::new(
        BrushStyle::default(),
        [BrushSample {
            x: 3.5,
            y: 3.5,
            pressure: 1.0,
        }],
    )
    .unwrap();
    let mut program = BrushProgram::new(8, 8)
        .unwrap()
        .append(paint_stroke)
        .unwrap();
    let strokes = Arc::make_mut(&mut program.strokes);
    strokes[0] = strokes[0].as_current_clone().unwrap();
    let mut document = Document::new("Forged CurrentClone snapshot", 8, 8);
    document.layers.push(Layer {
        id: 1,
        name: "Forged Paint".into(),
        kind: LayerKind::Paint { program },
        ..Layer::default()
    });
    document.next_id = 2;
    document
}

#[test]
fn authoring_only_current_clone_is_rejected_from_every_snapshot_version() {
    for version in [7, 10, 11] {
        let mut forged = document_with_authoring_only_clone_marker();
        forged.version = version;
        let error = forged.migrate().unwrap_err();
        assert!(
            format!("{error:#}").contains("authoring-only CurrentClone"),
            "snapshot v{version} returned {error:#}"
        );

        let encoded = serde_json::to_string(&forged).unwrap();
        let error = serde_json::from_str::<Document>(&encoded).unwrap_err();
        assert!(
            error.to_string().contains("authoring-only CurrentClone"),
            "encoded snapshot v{version} returned {error}"
        );
    }
}

#[test]
fn authoring_only_current_clone_is_rejected_from_every_paint_transfer_version() {
    for version in [5, 9] {
        let document = document_with_authoring_only_clone_marker();
        let mut transfer = LayerTransfer {
            format: LAYER_TRANSFER_FORMAT.into(),
            version,
            layer: document.layers[0].clone(),
            font_asset: None,
            sampled_sources: std::collections::BTreeMap::new(),
        };
        transfer.layer.id = 0;
        let error = transfer.to_json().unwrap_err();
        assert!(
            format!("{error:#}").contains("authoring-only CurrentClone"),
            "transfer v{version} returned {error:#}"
        );

        let encoded = serde_json::to_string(&transfer).unwrap();
        let error = LayerTransfer::from_json(&encoded).unwrap_err();
        assert!(
            format!("{error:#}").contains("authoring-only CurrentClone"),
            "encoded transfer v{version} returned {error:#}"
        );
    }
}

#[test]
fn forged_v14_current_clone_operation_is_rejected_before_replay() {
    use sha2::{Digest, Sha256};

    let directory = test_directory("forged-current-clone");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let project = directory.join("forged.prism");
    let session_id = spectrum_revisions::SessionId::new();
    write_source(&source);
    let mut workspace = Workspace::create_durable(
        source_document(&source),
        &project,
        spectrum_revisions::Actor {
            id: "person:forged-clone".into(),
            display_name: "Forged Clone tester".into(),
            kind: spectrum_revisions::ActorKind::Human,
        },
        session_id,
    )
    .unwrap();
    workspace
        .execute(Command::SetCloneSource {
            id: 1,
            document_x: 1.5,
            document_y: 1.5,
            resolved_source: None,
        })
        .unwrap();
    let paint_id = workspace
        .execute(Command::AddPaintLayer {
            name: Some("Clone".into()),
            width: 8,
            height: 8,
        })
        .unwrap()
        .layer_ids[0];
    workspace
        .execute(Command::AddBrushStroke {
            id: paint_id,
            stroke: clone_stroke(vec![sample(4.5, 4.5)], 1.0),
            selection: PaintSelection::None,
        })
        .unwrap();
    workspace.checkpoint().unwrap();
    drop(workspace);

    let live_database = fs::read_dir(directory.join(".revision-cache"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("live.sqlite");
    for database in [&project, &live_database] {
        let connection = rusqlite::Connection::open(database).unwrap();
        let (revision_id, version, bytes): (Vec<u8>, u32, Vec<u8>) = connection
            .query_row(
                "SELECT revision_id, version, bytes FROM operation_payloads
                 WHERE instr(CAST(bytes AS TEXT), 'add_brush_stroke') > 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(version, 14);
        let mut commands: Vec<Command> = serde_json::from_slice(&bytes).unwrap();
        let Command::AddBrushStroke { stroke, .. } = &mut commands[0] else {
            panic!("expected one durable AddBrushStroke command");
        };
        *stroke = stroke.as_current_clone().unwrap();
        let forged = serde_json::to_vec(&commands).unwrap();
        let digest: [u8; 32] = Sha256::digest(&forged).into();
        connection
            .execute(
                "UPDATE operation_payloads SET bytes = ?1, sha256 = ?2
                 WHERE revision_id = ?3",
                rusqlite::params![forged, digest.as_slice(), revision_id],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM snapshots WHERE revision_id = ?1",
                rusqlite::params![revision_id],
            )
            .unwrap();
        let remaining_snapshot: u32 = connection
            .query_row(
                "SELECT count(*) FROM snapshots WHERE revision_id = ?1",
                rusqlite::params![revision_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_snapshot, 0);
    }

    let error = match Workspace::open_session(&project, session_id) {
        Ok(_) => panic!("forged CurrentClone operation unexpectedly replayed"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("authoring-only CurrentClone"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn in_project_clone_original_is_content_addressed_before_mutation_and_deletion() {
    let directory = test_directory("in-project-portable");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let project = directory.join("portable.prism");
    write_source(&source);
    let mut workspace = Workspace::new(source_document(&source), None);
    workspace
        .execute(Command::SetCloneSource {
            id: 1,
            document_x: 1.5,
            document_y: 1.5,
            resolved_source: None,
        })
        .unwrap();
    workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: Some("Clone".into()),
            width: 8,
            height: 8,
            stroke: clone_stroke(vec![sample(4.5, 4.5), sample(5.5, 5.5)], 2.0),
            selection: PaintSelection::None,
        })
        .unwrap();
    workspace.execute(Command::RemoveLayer { id: 1 }).unwrap();
    let expected = render_document(&workspace.document, None)
        .unwrap()
        .to_rgba8();
    save_document(&workspace.document, &project).unwrap();

    let encoded = fs::read_to_string(&project).unwrap();
    assert!(encoded.contains("portable-assets/sampled-"));
    assert!(!encoded.contains("\"path\": \"source.png\""));
    RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255]))
        .save(&source)
        .unwrap();
    fs::remove_file(&source).unwrap();

    let reopened = load_document(&project).unwrap();
    let sampled = reopened.sampled_sources.values().next().unwrap();
    assert!(sampled.path.exists());
    assert_ne!(sampled.path, source);
    assert_eq!(
        render_document(&reopened, None).unwrap().to_rgba8(),
        expected
    );
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn export_cannot_replace_the_sole_sampled_asset_after_source_layer_removal() {
    use std::os::unix::fs::PermissionsExt;

    let directory = test_directory("export-source-refusal");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("sampled.png");
    write_source(&source);
    fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
    let mut workspace = Workspace::new(source_document(&source), None);
    workspace
        .execute(Command::SetCloneSource {
            id: 1,
            document_x: 1.5,
            document_y: 1.5,
            resolved_source: None,
        })
        .unwrap();
    workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: Some("Clone".into()),
            width: 8,
            height: 8,
            stroke: clone_stroke(vec![sample(4.5, 4.5)], 2.0),
            selection: PaintSelection::None,
        })
        .unwrap();
    workspace.execute(Command::RemoveLayer { id: 1 }).unwrap();
    let before = fs::read(&source).unwrap();
    let before_mode = fs::metadata(&source).unwrap().permissions().mode();

    assert!(export_document(&workspace.document, &source, 92).is_err());
    assert_eq!(fs::read(&source).unwrap(), before);
    assert_eq!(
        fs::metadata(&source).unwrap().permissions().mode(),
        before_mode
    );
    assert!(render_document(&workspace.document, None).is_ok());
    fs::remove_dir_all(directory).unwrap();
}
