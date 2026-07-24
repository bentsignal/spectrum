use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
};

use spectrum_revisions::{
    Actor, ActorKind, AssetId, CollaborationMode, Revision, RevisionId, RevisionStore, SessionId,
};
use ttf_parser::Face;

use crate::{
    BlendMode, Command, Document, LayerKind, LayerTransfer, TextTypography, Workspace,
    rasterize_shape_asset,
};

use super::*;

const STATIC_FONT: &[u8] =
    include_bytes!("../../../crates/spectrum-fonts/tests/fonts/noto-sans-static-source.ttf");
const LAYOUT_FONT: &[u8] =
    include_bytes!("../../../crates/spectrum-fonts/tests/fonts/noto-sans-layout-source.ttf");

fn actor() -> Actor {
    Actor {
        id: "test:optimized-copy".into(),
        display_name: "Optimized Copy Test".into(),
        kind: ActorKind::Human,
    }
}

fn directory(label: &str) -> PathBuf {
    fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!("prism-{label}-{}", SessionId::new()))
}

fn project_with_historical_font_usage(directory: &Path) -> (PathBuf, String, usize) {
    fs::create_dir_all(directory).unwrap();
    let source = directory.join("source.prism");
    let font_path = directory.join("NotoSans.ttf");
    fs::write(&font_path, STATIC_FONT).unwrap();
    let mut workspace = Workspace::create_durable(
        Document::new("Optimized copy", 320, 200),
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
            text: "AV".into(),
            name: None,
            font_size: 36.0,
            color: [255; 4],
            x: 4.0,
            y: 8.0,
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
    workspace
        .execute(Command::UpdateText {
            id: layer_id,
            text: "A".into(),
            font_size: 36.0,
            color: [255; 4],
        })
        .unwrap();
    let revision_count = workspace.history().unwrap().unwrap().revisions.len();
    drop(workspace);
    (source, source_hash, revision_count)
}

fn replay_document(store: &RevisionStore, revision: RevisionId) -> Document {
    let plan = store.replay_plan(revision, &PrismCompatibility).unwrap();
    let mut document: Document =
        serde_json::from_slice(&decode_snapshot(&plan.snapshot).unwrap()).unwrap();
    readonly_font::hydrate_read_only_legacy_font_permissions(store, &mut document).unwrap();
    document.migrate().unwrap();
    for step in plan.steps {
        let commands: Vec<Command> = serde_json::from_slice(&step.operations.bytes).unwrap();
        validate_operations_version(&commands, step.operations.encoding.version).unwrap();
        replay_commands(store, &mut document, commands).unwrap();
    }
    document
}

fn prove_sequential_operation_replay(
    store: &RevisionStore,
    revisions: &[Revision],
    proof_path: &Path,
) {
    let mut replayed = replay_document(store, revisions[0].id);
    for (index, revision) in revisions[1..].iter().enumerate() {
        let (_, commands) = operation_batch(store, revision).unwrap();
        replay_commands(store, &mut replayed, commands).unwrap();
        let exact_snapshot = replay_document(store, revision.id);
        assert_eq!(
            replayed, exact_snapshot,
            "sequential operation replay diverged at rewritten revision {}",
            revision.id
        );
        validate_render_parity(
            store,
            &replayed,
            store,
            &exact_snapshot,
            &proof_path.with_file_name(format!("sequential-state-{index}.prism")),
        )
        .unwrap();
    }
}

fn prove_navigation_reopen_and_undo(path: &Path, revisions: &[RevisionId]) {
    let session = SessionId::new();
    for revision in revisions {
        let mut workspace = Workspace::open_as(path, actor(), session).unwrap();
        workspace.move_to_revision(*revision).unwrap();
        let expected = workspace.document.clone();
        drop(workspace);
        let reopened = Workspace::open_session(path, session).unwrap();
        assert_eq!(reopened.document, expected);
    }
    let mut workspace = Workspace::open_as(path, actor(), session).unwrap();
    workspace
        .move_to_revision(*revisions.last().unwrap())
        .unwrap();
    let tip = workspace.document.clone();
    workspace.execute(Command::Undo).unwrap();
    let parent = workspace.document.clone();
    assert_ne!(parent, tip);
    workspace.execute(Command::Redo).unwrap();
    assert_eq!(workspace.document, tip);
}

#[test]
fn distinct_sources_cannot_converge_to_one_subset_identity() {
    let mut subset_sources = std::collections::HashMap::new();
    require_unique_subset_content(&mut subset_sources, "bbbb", "subset").unwrap();
    require_unique_subset_content(&mut subset_sources, "bbbb", "subset").unwrap();

    let error = require_unique_subset_content(&mut subset_sources, "aaaa", "subset").unwrap_err();

    assert_eq!(
        error.to_string(),
        "distinct source fonts aaaa and bbbb converge to subset content hash subset; optimized copy cannot preserve font identity"
    );
}

#[test]
fn runtime_validation_rejects_exact_snapshots_that_mask_nonreplayable_operations() {
    let directory = directory("optimized-masked-replay");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("masked.prism");
    let session_id = SessionId::new();
    let root_document = Document::new("Root", 32, 32);
    let root_snapshot = PreparedSnapshot::portable(&root_document).unwrap();
    let (mut store, info) = RevisionStore::create(
        &path,
        spectrum_revisions::NewProject {
            application_id: APPLICATION_ID.into(),
            application_version: env!("CARGO_PKG_VERSION").into(),
            actor: actor(),
            session_id,
            root_label: Some("Root".into()),
            track_kind: "document".into(),
            track_label: "Document".into(),
            initial_snapshots: vec![root_snapshot.payload],
            assets: root_snapshot.assets,
        },
    )
    .unwrap();
    let commands = vec![Command::RenameDocument {
        name: "Only operations know this name".into(),
    }];
    let masked_snapshot = PreparedSnapshot::portable(&root_document).unwrap();
    store
        .append(spectrum_revisions::AppendRevision {
            track_id: info.default_track_id,
            session_id,
            expected_parent: info.root_revision,
            application_version: env!("CARGO_PKG_VERSION").into(),
            label: Some("Masked divergence".into()),
            command_count: 1,
            operation_payloads: vec![spectrum_revisions::Payload::new(
                spectrum_revisions::Encoding::new(
                    OPERATIONS_FAMILY,
                    crate::revisions::DOCUMENT_LIFECYCLE_OPERATIONS_VERSION,
                ),
                serde_json::to_vec(&commands).unwrap(),
            )],
            snapshots: vec![masked_snapshot.payload],
            assets: masked_snapshot.assets,
        })
        .unwrap();
    store.verify_integrity().unwrap();

    let error = validate_copy_store(&store, 2).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("operations do not reproduce its exact snapshot")
    );
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn rewrites_linear_history_with_union_font_repertoire_and_exact_snapshots() {
    let directory = directory("optimized-history");
    let (source, source_hash, revision_count) = project_with_historical_font_usage(&directory);
    let source_store = RevisionStore::open_read_only(&source).unwrap();
    let (source_revisions, source_track) = linear_history(&source_store, &source).unwrap();
    let source_tip = source_revisions.last().unwrap().id;
    let source_root_session = source_revisions[0].session_id;
    drop(source_store);
    let follower = Workspace::start_collaboration(
        &source,
        Some(source_root_session),
        Actor {
            id: "test:follower".into(),
            display_name: "Follower".into(),
            kind: ActorKind::Agent,
        },
        CollaborationMode::Separate,
    )
    .unwrap();
    let output = directory.join("optimized.prism");
    let second_output = directory.join("optimized-again.prism");
    let source_before = fs::read(&source).unwrap();

    let report = create_optimized_font_copy(&source, &output).unwrap();
    let second_report = create_optimized_font_copy(&source, &second_output).unwrap();

    assert_eq!(fs::read(&source).unwrap(), source_before);
    assert_eq!(report.revisions, revision_count);
    assert_eq!(report.fonts.len(), 1);
    assert!(report.fonts[0].subset_bytes < report.fonts[0].source_bytes);
    assert!(report.output_bytes < report.source_bytes);
    assert_eq!(report.fonts, second_report.fonts);
    let destination = RevisionStore::open_read_only(&output).unwrap();
    destination.verify_integrity().unwrap();
    let info = destination.project_info().unwrap();
    let revisions = destination
        .revisions_for_track(info.default_track_id)
        .unwrap();
    assert_eq!(revisions.len(), revision_count);
    let (destination_revisions, destination_track) = linear_history(&destination, &output).unwrap();
    assert_eq!(destination_revisions, revisions);
    prove_sequential_operation_replay(&destination, &destination_revisions, &output);
    for (index, (source_revision, revision)) in source_revisions
        .iter()
        .zip(&destination_revisions)
        .enumerate()
    {
        assert_eq!(revision.actor, source_revision.actor);
        assert_eq!(revision.session_id, source_revision.session_id);
        assert_eq!(revision.label, source_revision.label);
        assert_eq!(revision.command_count, source_revision.command_count);
        assert_eq!(
            revision.application_version,
            source_revision.application_version
        );
        let plan = destination
            .replay_plan(revision.id, &PrismCompatibility)
            .unwrap();
        assert_eq!(plan.snapshot_revision, revision.id);
        assert!(plan.steps.is_empty());
        if revision.parent_id.is_some() {
            let operations = destination
                .compatible_operation_payload(revision.id, &PrismCompatibility)
                .unwrap()
                .unwrap();
            assert!(
                !String::from_utf8(operations.bytes)
                    .unwrap()
                    .contains(&source_hash)
            );
        }
        let source_document = replay_document(
            &RevisionStore::open_read_only(&source).unwrap(),
            source_revision.id,
        );
        let destination_document = replay_document(&destination, revision.id);
        validate_render_parity(
            &RevisionStore::open_read_only(&source).unwrap(),
            &source_document,
            &destination,
            &destination_document,
            &directory.join(format!("state-{index}.prism")),
        )
        .unwrap();
    }
    let author_sessions = destination.sessions_on_track(destination_track.id).unwrap();
    let expected_authors = source_revisions
        .iter()
        .map(|revision| revision.session_id)
        .collect::<HashSet<_>>();
    assert_eq!(
        author_sessions
            .iter()
            .map(|session| session.id)
            .collect::<HashSet<_>>(),
        expected_authors
    );
    assert!(
        author_sessions
            .iter()
            .all(|session| session.cursor == destination_revisions.last().unwrap().id)
    );
    assert!(
        !author_sessions
            .iter()
            .any(|session| session.id == follower.agent_session)
    );
    assert!(
        destination
            .collaboration(follower.agent_session)
            .unwrap()
            .is_none()
    );
    assert!(
        destination
            .asset_record(AssetId::from_hex(&source_hash).unwrap())
            .unwrap()
            .is_none()
    );
    let subset_hash = &report.fonts[0].content_hash;
    let subset = destination
        .asset_record(AssetId::from_hex(subset_hash).unwrap())
        .unwrap()
        .unwrap();
    assert!(
        Face::parse(&subset.bytes, 0)
            .unwrap()
            .glyph_index('V')
            .is_some()
    );
    let source_store = RevisionStore::open_read_only(&source).unwrap();
    assert!(
        source_store
            .asset_record(AssetId::from_hex(&source_hash).unwrap())
            .unwrap()
            .is_some()
    );
    drop(source_store);
    drop(destination);
    prove_navigation_reopen_and_undo(
        &source,
        &source_revisions
            .iter()
            .map(|revision| revision.id)
            .collect::<Vec<_>>(),
    );
    prove_navigation_reopen_and_undo(
        &output,
        &destination_revisions
            .iter()
            .map(|revision| revision.id)
            .collect::<Vec<_>>(),
    );
    let mut source_workspace = Workspace::open_as(&source, actor(), SessionId::new()).unwrap();
    let text_id = source_workspace.document.layers.last().unwrap().id;
    source_workspace
        .execute(Command::UpdateText {
            id: text_id,
            text: "C".into(),
            font_size: 36.0,
            color: [255; 4],
        })
        .unwrap();
    assert_eq!(
        source_workspace.document.font_assets[0].content_hash,
        source_hash
    );
    assert_eq!(source_track.id, source_revisions[0].track_id);
    assert_eq!(source_tip, source_revisions.last().unwrap().id);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn refuses_existing_destination_without_changing_its_bytes() {
    let directory = directory("optimized-no-overwrite");
    let (source, _, _) = project_with_historical_font_usage(&directory);
    let output = directory.join("sentinel.prism");
    fs::write(&output, b"do not overwrite").unwrap();

    let error = create_optimized_font_copy(&source, &output).unwrap_err();

    assert!(error.to_string().contains("refusing to overwrite"));
    assert_eq!(fs::read(&output).unwrap(), b"do not overwrite");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn refuses_a_project_without_embedded_fonts() {
    let directory = directory("optimized-no-fonts");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.prism");
    let output = directory.join("optimized.prism");
    let workspace = Workspace::create_durable(
        Document::new("No fonts", 32, 24),
        &source,
        actor(),
        SessionId::new(),
    )
    .unwrap();
    drop(workspace);

    let error = create_optimized_font_copy(&source, &output).unwrap_err();

    assert!(error.to_string().contains("no embedded fonts"));
    assert!(!output.exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mixed_subsettable_and_restricted_fonts_fail_without_any_output() {
    let directory = directory("optimized-mixed-font-policy");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.prism");
    let supported = directory.join("Supported.ttf");
    let restricted = directory.join("Restricted.ttf");
    fs::write(&supported, STATIC_FONT).unwrap();
    let mut restricted_bytes = epaint_default_fonts::HACK_REGULAR.to_vec();
    set_fs_type(&mut restricted_bytes, 0x0002);
    fs::write(&restricted, restricted_bytes).unwrap();
    let mut workspace = Workspace::create_durable(
        Document::new("Mixed font policy", 96, 64),
        &source,
        actor(),
        SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: supported,
            source_name: None,
        })
        .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: restricted,
            source_name: None,
        })
        .unwrap();
    let supported_id = workspace.document.font_assets[0].id;
    workspace
        .execute(Command::AddText {
            text: "AV".into(),
            name: None,
            font_size: 24.0,
            color: [255; 4],
            x: 2.0,
            y: 2.0,
        })
        .unwrap();
    let text = workspace.document.selected.unwrap();
    workspace
        .execute(Command::SetTextTypography {
            id: text,
            typography: TextTypography {
                font_id: Some(supported_id),
                ..Default::default()
            },
        })
        .unwrap();
    drop(workspace);
    let source_before = fs::read(&source).unwrap();
    let output = directory.join("optimized.prism");

    let error = create_optimized_font_copy(&source, &output).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("embedding metadata forbids subsetting")
    );
    assert_eq!(fs::read(&source).unwrap(), source_before);
    assert!(!output.exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn rewrites_every_path_bearing_operation_and_preserves_non_font_assets() {
    let directory = directory("optimized-operation-assets");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.prism");
    let font_path = directory.join("NotoSans.ttf");
    let layout_font_path = directory.join("NotoSansLayout.ttf");
    let added_raster_path = directory.join("added.png");
    let transferred_raster_path = directory.join("transferred.png");
    fs::write(&font_path, STATIC_FONT).unwrap();
    fs::write(&layout_font_path, LAYOUT_FONT).unwrap();
    image::RgbaImage::from_pixel(6, 5, image::Rgba([20, 40, 60, 255]))
        .save(&added_raster_path)
        .unwrap();
    image::RgbaImage::from_pixel(4, 3, image::Rgba([80, 100, 120, 255]))
        .save(&transferred_raster_path)
        .unwrap();

    let mut raster_source = Workspace::new(Document::new("Raster transfer", 20, 20), None);
    raster_source
        .execute(Command::AddRaster {
            path: transferred_raster_path.clone(),
            name: Some("Transferred raster".into()),
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    let raster_transfer = LayerTransfer::from_selected(&raster_source.document).unwrap();

    let mut text_source = Workspace::new(Document::new("Font transfer", 80, 40), None);
    text_source
        .execute(Command::ImportFont {
            path: font_path.clone(),
            source_name: None,
        })
        .unwrap();
    let transfer_font_id = text_source.document.font_assets[0].id;
    text_source
        .execute(Command::AddText {
            text: "AV".into(),
            name: Some("Transferred text".into()),
            font_size: 24.0,
            color: [255; 4],
            x: 1.0,
            y: 1.0,
        })
        .unwrap();
    text_source
        .execute(Command::SetTextTypography {
            id: text_source.document.selected.unwrap(),
            typography: TextTypography {
                font_id: Some(transfer_font_id),
                ..Default::default()
            },
        })
        .unwrap();
    let font_transfer = LayerTransfer::from_selected(&text_source.document).unwrap();

    let mut layout_text_source =
        Workspace::new(Document::new("Layout font transfer", 80, 40), None);
    layout_text_source
        .execute(Command::ImportFont {
            path: layout_font_path.clone(),
            source_name: None,
        })
        .unwrap();
    let layout_font_id = layout_text_source.document.font_assets[0].id;
    layout_text_source
        .execute(Command::AddText {
            text: "ffiAVÅ".into(),
            name: Some("Transferred layout text".into()),
            font_size: 24.0,
            color: [255; 4],
            x: 1.0,
            y: 32.0,
        })
        .unwrap();
    layout_text_source
        .execute(Command::SetTextTypography {
            id: layout_text_source.document.selected.unwrap(),
            typography: TextTypography {
                font_id: Some(layout_font_id),
                ..Default::default()
            },
        })
        .unwrap();
    let layout_font_transfer = LayerTransfer::from_selected(&layout_text_source.document).unwrap();

    let mut shape_source = Workspace::new(Document::new("Shape raster", 40, 30), None);
    shape_source
        .execute(Command::AddRectangle {
            name: Some("Rasterized".into()),
            width: 12,
            height: 8,
            color: [180, 20, 40, 255],
            corner_radius: 2.0,
            x: 2.0,
            y: 3.0,
        })
        .unwrap();
    let rasterized = rasterize_shape_asset(&shape_source.document, 1, 1.0).unwrap();
    let rasterized_bytes = fs::read(&rasterized.path).unwrap();

    let mut workspace = Workspace::create_durable(
        Document::new("Operation assets", 120, 90),
        &source,
        actor(),
        SessionId::new(),
    )
    .unwrap();
    workspace
        .execute_batch(vec![
            Command::ImportFont {
                path: font_path,
                source_name: None,
            },
            Command::ImportFont {
                path: layout_font_path,
                source_name: None,
            },
            Command::AddRaster {
                path: added_raster_path.clone(),
                name: Some("Added raster".into()),
                x: 0.0,
                y: 0.0,
            },
            Command::SetBlendMode {
                id: 1,
                blend_mode: BlendMode::Dissolve,
            },
            Command::SetDissolveSeed {
                id: 1,
                seed: 0x1234_5678,
            },
            Command::AddRectangle {
                name: Some("Rasterized".into()),
                width: 12,
                height: 8,
                color: [180, 20, 40, 255],
                corner_radius: 2.0,
                x: 2.0,
                y: 3.0,
            },
            Command::RasterizeShape {
                id: 2,
                path: rasterized.path,
                scale: rasterized.scale,
            },
            Command::InsertLayer {
                transfer: Box::new(raster_transfer),
                index: None,
            },
            Command::InsertLayer {
                transfer: Box::new(font_transfer),
                index: None,
            },
            Command::InsertLayer {
                transfer: Box::new(layout_font_transfer),
                index: None,
            },
        ])
        .unwrap();
    let source_font_hashes = workspace
        .document
        .font_assets
        .iter()
        .map(|font| font.content_hash.clone())
        .collect::<Vec<_>>();
    drop(workspace);
    let expected_rasters = [
        fs::read(added_raster_path).unwrap(),
        fs::read(transferred_raster_path).unwrap(),
        rasterized_bytes,
    ];
    let output = directory.join("optimized.prism");

    create_optimized_font_copy(&source, &output).unwrap();

    let destination = RevisionStore::open_read_only(&output).unwrap();
    let info = destination.project_info().unwrap();
    let revisions = destination
        .revisions_for_track(info.default_track_id)
        .unwrap();
    let (ordered, _) = linear_history(&destination, &output).unwrap();
    prove_sequential_operation_replay(&destination, &ordered, &output);
    for revision in &revisions {
        replay_document(&destination, revision.id);
        if revision.parent_id.is_some() {
            let payload = destination
                .compatible_operation_payload(revision.id, &PrismCompatibility)
                .unwrap()
                .unwrap();
            assert_eq!(
                payload.encoding.version,
                crate::revisions::DISSOLVE_OPERATIONS_VERSION
            );
            let json = String::from_utf8(payload.bytes).unwrap();
            assert!(
                source_font_hashes
                    .iter()
                    .all(|source_hash| !json.contains(source_hash))
            );
            let commands: Vec<Command> = serde_json::from_str(&json).unwrap();
            assert!(
                commands
                    .iter()
                    .any(|command| matches!(command, Command::AddRaster { .. }))
            );
            assert!(
                commands
                    .iter()
                    .any(|command| matches!(command, Command::RasterizeShape { .. }))
            );
            assert!(
                commands
                    .iter()
                    .any(|command| matches!(command, Command::ImportFont { .. }))
            );
            assert!(commands.iter().any(|command| {
                matches!(command, Command::InsertLayer { transfer, .. }
                    if matches!(transfer.layer.kind, LayerKind::Raster { .. }))
            }));
            assert!(commands.iter().any(|command| {
                matches!(command, Command::InsertLayer { transfer, .. }
                    if transfer.font_asset.is_some())
            }));
        }
    }
    for source_hash in source_font_hashes {
        assert!(
            destination
                .asset_record(AssetId::from_hex(&source_hash).unwrap())
                .unwrap()
                .is_none()
        );
    }
    for bytes in expected_rasters {
        let id = AssetId::for_bytes(&bytes);
        assert_eq!(destination.asset_record(id).unwrap().unwrap().bytes, bytes);
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn publishes_to_a_fresh_destination_directory() {
    let source_directory = directory("optimized-source-directory");
    let output_directory = directory("optimized-output-directory");
    let (source, _, _) = project_with_historical_font_usage(&source_directory);
    fs::create_dir_all(&output_directory).unwrap();
    let output = output_directory.join("optimized.prism");

    let report = create_optimized_font_copy(&source, &output).unwrap();

    assert_eq!(report.output, fs::canonicalize(&output).unwrap());
    assert!(output.is_file());
    fs::remove_dir_all(source_directory).unwrap();
    fs::remove_dir_all(output_directory).unwrap();
}

#[test]
fn atomic_publish_loses_a_destination_creation_race_without_clobbering() {
    let directory = directory("optimized-race");
    let (source, _, _) = project_with_historical_font_usage(&directory);
    let output = directory.join("raced.prism");

    let error = create_optimized_font_copy_before_publish(&source, &output, || {
        fs::write(&output, b"racing writer wins")?;
        Ok(())
    })
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("could not publish optimized copy")
    );
    assert_eq!(fs::read(&output).unwrap(), b"racing writer wins");
    assert!(fs::read_dir(&directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("optimized-copy.tmp")
    }));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn post_rename_sync_failure_reports_that_the_destination_exists() {
    let directory = directory("optimized-sync-failure");
    fs::create_dir_all(&directory).unwrap();
    let temporary = directory.join("private.prism");
    let output = directory.join("published.prism");
    let mut cleanup = TemporaryProject::create(temporary.clone()).unwrap();
    fs::write(&temporary, b"published bytes").unwrap();

    let error = publish_optimized_copy(&temporary, &output, &mut cleanup, |source, destination| {
        fs::rename(source, destination)?;
        Err(spectrum_revisions::RevisionError::PublishedButNotSynced {
            destination: destination.to_owned(),
            source: std::io::Error::other("injected sync failure"),
        })
    })
    .unwrap_err();

    assert!(error.to_string().contains("optimized copy exists at"));
    drop(cleanup);
    assert_eq!(fs::read(output).unwrap(), b"published bytes");
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
fn source_change_at_publish_boundary_aborts_without_destination() {
    let directory = directory("optimized-source-race");
    let (source, _, _) = project_with_historical_font_usage(&directory);
    let output = directory.join("optimized.prism");

    let error = create_optimized_font_copy_before_publish(&source, &output, || {
        OpenOptions::new()
            .append(true)
            .open(&source)?
            .write_all(b"changed")?;
        Ok(())
    })
    .unwrap_err();

    assert!(error.to_string().contains("source Prism project changed"));
    assert!(!output.exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn refuses_branched_history_without_publishing_a_destination() {
    let directory = directory("optimized-branch");
    let (source, _, _) = project_with_historical_font_usage(&directory);
    let mut workspace = Workspace::open_as(&source, actor(), SessionId::new()).unwrap();
    let history = workspace.history().unwrap().unwrap();
    workspace.move_to_revision(history.revisions[2].id).unwrap();
    workspace
        .execute(Command::RenameDocument {
            name: "Alternate".into(),
        })
        .unwrap();
    drop(workspace);
    let output = directory.join("optimized.prism");

    let error = create_optimized_font_copy(&source, &output).unwrap_err();

    assert!(error.to_string().contains("without branches"));
    assert!(!output.exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn refuses_linear_history_when_active_cursor_is_undone_from_the_tip() {
    let directory = directory("optimized-undone");
    let (source, _, _) = project_with_historical_font_usage(&directory);
    let mut workspace = Workspace::open_as(&source, actor(), SessionId::new()).unwrap();
    let history = workspace.history().unwrap().unwrap();
    workspace.move_to_revision(history.revisions[2].id).unwrap();
    drop(workspace);
    let output = directory.join("optimized.prism");

    let error = create_optimized_font_copy(&source, &output).unwrap_err();

    assert!(error.to_string().contains("active project cursor"));
    assert!(!output.exists());
    fs::remove_dir_all(directory).unwrap();
}
