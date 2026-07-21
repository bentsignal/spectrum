use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use image::{Rgba, RgbaImage};
use rusqlite::Connection;
use spectrum_revisions::{Actor, ActorKind, AssetId, SessionId};

use crate::{
    Command, Document, DurableProject, LayerKind, Transform, Workspace, apply_command,
    export_document, render_document,
};

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-durable-asset-{label}-{stamp}"))
}

fn test_actor(label: &str) -> Actor {
    Actor {
        id: format!("person:{label}"),
        display_name: label.into(),
        kind: ActorKind::Human,
    }
}

fn save_pixel(path: &Path, pixel: [u8; 4]) {
    RgbaImage::from_pixel(4, 4, Rgba(pixel)).save(path).unwrap();
}

fn raster_paths(document: &Document) -> (&Path, Option<&Path>) {
    let LayerKind::Raster {
        path,
        original_path,
    } = &document.layers[0].kind
    else {
        panic!("expected raster layer");
    };
    (path, original_path.as_deref())
}

fn count_rows(project: &Path, table: &str) -> i64 {
    let connection = Connection::open(project).unwrap();
    connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn durable_raster_uses_one_staged_snapshot_and_stable_name_across_history() {
    let directory = test_directory("stable");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source-name.png");
    let committed_pixel = [12, 34, 56, 255];
    save_pixel(&source, committed_pixel);
    let committed_bytes = fs::read(&source).unwrap();
    let canonical_source = fs::canonicalize(&source).unwrap();
    let project = directory.join("stable.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Stable raster", 4, 4),
        &project,
        test_actor("stable"),
        SessionId::new(),
    )
    .unwrap();

    workspace
        .execute(Command::AddRaster {
            path: source.clone(),
            name: None,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    assert_eq!(workspace.document.layers[0].name, "source-name");
    let (staged_path, original_path) = raster_paths(&workspace.document);
    assert_eq!(original_path, Some(canonical_source.as_path()));
    assert_ne!(staged_path, canonical_source);
    assert_eq!(fs::read(staged_path).unwrap(), committed_bytes);
    let asset_id = AssetId::for_bytes(&committed_bytes);
    assert_eq!(
        staged_path.file_name(),
        Some(OsStr::new(&format!("{asset_id}.png")))
    );
    let staged_cache = staged_path.parent().unwrap().to_owned();

    save_pixel(&source, [200, 1, 2, 255]);
    assert!(export_document(&workspace.document, &source, 90).is_err());
    fs::remove_file(&source).unwrap();
    assert_eq!(
        render_document(&workspace.document, None)
            .unwrap()
            .to_rgba8()
            .get_pixel(0, 0)
            .0,
        committed_pixel
    );

    let raster_id = workspace.document.layers[0].id;
    workspace
        .execute_batch(
            (1..=99)
                .map(|x| Command::SetTransform {
                    id: raster_id,
                    transform: Transform {
                        x: x as f32,
                        ..Default::default()
                    },
                })
                .collect(),
        )
        .unwrap();
    workspace.checkpoint().unwrap();
    drop(workspace);

    assert_eq!(count_rows(&project, "snapshots"), 2);
    let connection = Connection::open(&project).unwrap();
    let embedded_bytes: Vec<u8> = connection
        .query_row("SELECT bytes FROM assets", [], |row| row.get(0))
        .unwrap();
    assert_eq!(embedded_bytes, committed_bytes);
    let operation_bytes: Vec<u8> = connection
        .query_row(
            "SELECT bytes FROM operation_payloads WHERE instr(CAST(bytes AS TEXT), 'add_raster') > 0 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(connection);
    let operation_json = String::from_utf8(operation_bytes).unwrap();
    assert!(operation_json.contains("spectrum-asset:"));
    assert!(!operation_json.contains(canonical_source.to_string_lossy().as_ref()));
    assert!(operation_json.contains("source-name"));

    let mut reopened =
        Workspace::open_as(&project, test_actor("reopen"), SessionId::new()).unwrap();
    assert_eq!(reopened.document.layers[0].name, "source-name");
    assert_eq!(reopened.document.layers[0].transform.x, 99.0);
    assert!(raster_paths(&reopened.document).1.is_none());
    reopened.execute(Command::Undo).unwrap();
    assert_eq!(reopened.document.layers[0].name, "source-name");
    assert_eq!(reopened.document.layers[0].transform.x, 0.0);
    assert!(raster_paths(&reopened.document).1.is_none());
    reopened.execute(Command::Undo).unwrap();
    assert!(reopened.document.layers.is_empty());
    reopened.execute(Command::Redo).unwrap();
    assert_eq!(reopened.document.layers[0].name, "source-name");
    assert!(raster_paths(&reopened.document).1.is_none());
    assert_eq!(
        render_document(&reopened.document, None)
            .unwrap()
            .to_rgba8()
            .get_pixel(0, 0)
            .0,
        committed_pixel
    );
    reopened.execute(Command::Redo).unwrap();
    assert_eq!(reopened.document.layers[0].transform.x, 99.0);
    drop(reopened);

    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(staged_cache).unwrap();
}

#[test]
fn failing_durable_asset_batch_changes_neither_document_history_nor_embedded_assets() {
    let directory = test_directory("atomic-failure");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("candidate.png");
    save_pixel(&source, [1, 2, 3, 255]);
    let project = directory.join("atomic.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Atomic", 4, 4),
        &project,
        test_actor("atomic"),
        SessionId::new(),
    )
    .unwrap();
    let before = workspace.document.clone();
    let history_before = workspace.history().unwrap().unwrap();
    let asset_count_before = count_rows(&project, "assets");

    let error = workspace
        .execute_batch(vec![
            Command::AddRaster {
                path: source,
                name: None,
                x: 0.0,
                y: 0.0,
            },
            Command::SetOpacity {
                id: 99,
                opacity: 0.5,
            },
        ])
        .unwrap_err();
    assert!(error.to_string().contains("layer 99"));
    assert_eq!(workspace.document, before);
    let history_after = workspace.history().unwrap().unwrap();
    assert_eq!(history_after.current, history_before.current);
    assert_eq!(
        history_after.revisions.len(),
        history_before.revisions.len()
    );
    assert_eq!(count_rows(&project, "assets"), asset_count_before);

    drop(workspace);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_asset_commands_use_only_the_atomic_workspace_path() {
    let directory = test_directory("entry-points");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("entry.png");
    save_pixel(&source, [9, 8, 7, 255]);
    let project = directory.join("entry.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Entry points", 4, 4),
        &project,
        test_actor("entry"),
        SessionId::new(),
    )
    .unwrap();

    workspace.begin_interaction();
    assert!(
        workspace
            .preview(Command::AddRaster {
                path: source.clone(),
                name: None,
                x: 0.0,
                y: 0.0,
            })
            .unwrap_err()
            .to_string()
            .contains("cannot preview")
    );
    assert!(workspace.cancel_interaction());
    workspace
        .execute_batch(vec![
            Command::AddRaster {
                path: source.clone(),
                name: None,
                x: 0.0,
                y: 0.0,
            },
            Command::SetOpacity {
                id: 1,
                opacity: 0.5,
            },
        ])
        .unwrap();
    assert_eq!(workspace.document.layers[0].opacity, 0.5);
    assert_eq!(workspace.history().unwrap().unwrap().revisions.len(), 2);
    assert_eq!(count_rows(&project, "assets"), 1);
    drop(workspace);

    let direct_project_path = directory.join("direct.prism");
    let (mut direct, mut document) = DurableProject::create(
        &direct_project_path,
        &Document::new("Direct", 4, 4),
        test_actor("direct"),
        SessionId::new(),
    )
    .unwrap();
    let command = Command::AddRaster {
        path: source,
        name: None,
        x: 0.0,
        y: 0.0,
    };
    apply_command(&mut document, command.clone()).unwrap();
    assert!(
        direct
            .commit(&[command], &document, "Unsafe direct import")
            .unwrap_err()
            .to_string()
            .contains("must be committed through")
    );
    assert_eq!(direct.history().unwrap().revisions.len(), 1);
    assert_eq!(count_rows(&direct_project_path, "assets"), 0);
    drop(direct);

    fs::remove_dir_all(directory).unwrap();
}
