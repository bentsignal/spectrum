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
    fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!("prism-durable-asset-{label}-{stamp}"))
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
fn oversized_raster_is_rejected_before_allocation_staging_or_history() {
    let directory = test_directory("oversized-raster");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("oversized.png");
    fs::File::create(&source)
        .unwrap()
        .set_len(crate::revisions::MAX_EMBEDDED_RASTER_BYTES as u64 + 1)
        .unwrap();
    let project = directory.join("oversized.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Oversized raster", 4, 4),
        &project,
        test_actor("oversized-raster"),
        SessionId::new(),
    )
    .unwrap();
    let history_before = workspace.history().unwrap().unwrap();
    let assets_before = count_rows(&project, "assets");

    let error = workspace
        .execute(Command::AddRaster {
            path: source,
            name: None,
            x: 0.0,
            y: 0.0,
        })
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("512 MiB"),
        "unexpected oversized-raster error: {error:#}"
    );
    assert!(workspace.document.layers.is_empty());
    assert_eq!(count_rows(&project, "assets"), assets_before);
    assert_eq!(
        workspace.history().unwrap().unwrap().current,
        history_before.current
    );
    drop(workspace);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn oversized_asset_batch_is_rejected_atomically_before_reads_or_staging() {
    let directory = test_directory("oversized-asset-batch");
    fs::create_dir_all(&directory).unwrap();
    let first = directory.join("first.png");
    let second = directory.join("second.png");
    fs::File::create(&first)
        .unwrap()
        .set_len(300 * 1024 * 1024)
        .unwrap();
    fs::File::create(&second)
        .unwrap()
        .set_len(300 * 1024 * 1024)
        .unwrap();
    let project = directory.join("batch.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Oversized asset batch", 4, 4),
        &project,
        test_actor("oversized-asset-batch"),
        SessionId::new(),
    )
    .unwrap();
    let history_before = workspace.history().unwrap().unwrap();
    let assets_before = count_rows(&project, "assets");

    let error = workspace
        .execute_batch(vec![
            Command::AddRaster {
                path: first,
                name: None,
                x: 0.0,
                y: 0.0,
            },
            Command::AddRaster {
                path: second,
                name: None,
                x: 0.0,
                y: 0.0,
            },
        ])
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("512 MiB across all assets"),
        "unexpected oversized-batch error: {error:#}"
    );
    assert!(workspace.document.layers.is_empty());
    assert_eq!(count_rows(&project, "assets"), assets_before);
    assert_eq!(
        workspace.history().unwrap().unwrap().current,
        history_before.current
    );
    drop(workspace);
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn durable_raster_rejects_symlinked_source_components() {
    use std::os::unix::fs::symlink;

    let directory = test_directory("linked-raster");
    let real_directory = directory.join("real");
    fs::create_dir_all(&real_directory).unwrap();
    let source = real_directory.join("image.png");
    save_pixel(&source, [1, 2, 3, 255]);
    let linked_file = directory.join("linked.png");
    let linked_directory = directory.join("linked-directory");
    symlink(&source, &linked_file).unwrap();
    symlink(&real_directory, &linked_directory).unwrap();
    let project = directory.join("linked.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Linked raster", 4, 4),
        &project,
        test_actor("linked-raster"),
        SessionId::new(),
    )
    .unwrap();

    for path in [linked_file, linked_directory.join("image.png")] {
        assert!(
            workspace
                .execute(Command::AddRaster {
                    path,
                    name: None,
                    x: 0.0,
                    y: 0.0,
                })
                .is_err()
        );
    }
    assert!(workspace.document.layers.is_empty());
    assert_eq!(count_rows(&project, "assets"), 0);
    drop(workspace);
    fs::remove_dir_all(directory).unwrap();
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
fn durable_font_import_rejects_oversize_before_staging_an_asset() {
    let directory = test_directory("oversized-font");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("oversized.ttf");
    fs::File::create(&source)
        .unwrap()
        .set_len(crate::font_source::MAX_EMBEDDED_FONT_BYTES as u64 + 1)
        .unwrap();
    let project = directory.join("oversized.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Bounded font", 4, 4),
        &project,
        test_actor("bounded-font"),
        SessionId::new(),
    )
    .unwrap();
    let history_before = workspace.history().unwrap().unwrap();
    let asset_count_before = count_rows(&project, "assets");

    let error = workspace
        .execute(Command::ImportFont {
            path: source,
            source_name: None,
        })
        .unwrap_err();

    assert!(error.to_string().contains("32 MiB"));
    assert!(workspace.document.font_assets.is_empty());
    assert_eq!(count_rows(&project, "assets"), asset_count_before);
    assert_eq!(
        workspace.history().unwrap().unwrap().current,
        history_before.current
    );
    drop(workspace);
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn durable_font_import_rejects_symlinked_final_and_ancestor_components() {
    use std::os::unix::fs::symlink;

    let directory = test_directory("linked-font");
    let real_directory = directory.join("real");
    fs::create_dir_all(&real_directory).unwrap();
    let source = real_directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let linked_directory = directory.join("linked-directory");
    let linked_file = directory.join("linked-font.ttf");
    symlink(&real_directory, &linked_directory).unwrap();
    symlink(&source, &linked_file).unwrap();
    let project = directory.join("linked.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Linked font", 4, 4),
        &project,
        test_actor("linked-font"),
        SessionId::new(),
    )
    .unwrap();
    let asset_count_before = count_rows(&project, "assets");

    for path in [linked_file, linked_directory.join("Hack-Regular.ttf")] {
        assert!(
            workspace
                .execute(Command::ImportFont {
                    path,
                    source_name: None,
                })
                .is_err()
        );
    }

    assert!(workspace.document.font_assets.is_empty());
    assert_eq!(count_rows(&project, "assets"), asset_count_before);
    drop(workspace);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_font_import_persists_the_verified_bytes_before_source_replacement() {
    let directory = test_directory("font-replacement");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let project = directory.join("font-replacement.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Font replacement", 4, 4),
        &project,
        test_actor("font-replacement"),
        SessionId::new(),
    )
    .unwrap();

    workspace
        .execute(Command::ImportFont {
            path: source.clone(),
            source_name: None,
        })
        .unwrap();
    fs::write(
        &source,
        vec![0x5a; epaint_default_fonts::HACK_REGULAR.len()],
    )
    .unwrap();
    drop(workspace);

    let loaded = Workspace::load_read_only(&project).unwrap();
    assert_eq!(loaded.font_assets.len(), 1);
    assert_eq!(
        loaded.font_assets[0].bytes().unwrap(),
        epaint_default_fonts::HACK_REGULAR
    );
    assert_eq!(count_rows(&project, "assets"), 1);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn periodic_snapshot_rejects_a_tampered_materialized_font() {
    let directory = test_directory("periodic-font-verification");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let project = directory.join("periodic-font.prism");
    let mut workspace = Workspace::create_durable(
        Document::new("Periodic font", 4, 4),
        &project,
        test_actor("periodic-font"),
        SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: source,
            source_name: None,
        })
        .unwrap();
    let embedded = workspace.document.font_assets[0].path.clone();
    fs::write(
        embedded,
        vec![0x7b; epaint_default_fonts::HACK_REGULAR.len()],
    )
    .unwrap();
    let before = workspace.document.clone();
    let history_before = workspace.history().unwrap().unwrap();
    let commands = (0..99)
        .map(|index| Command::SetCanvas {
            width: 4,
            height: 4,
            background: [index as u8, 0, 0, 255],
        })
        .collect();

    let error = workspace.execute_batch(commands).unwrap_err();

    assert!(error.to_string().contains("content identity"));
    assert_eq!(workspace.document, before);
    assert_eq!(
        workspace.history().unwrap().unwrap().current,
        history_before.current
    );
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
