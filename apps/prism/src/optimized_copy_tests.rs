use std::{
    fs::{self, OpenOptions},
    io::Write,
};

use spectrum_revisions::{Actor, ActorKind, AssetId, RevisionStore, SessionId};
use ttf_parser::Face;

use crate::{Command, Document, TextTypography, Workspace};

use super::*;

const STATIC_FONT: &[u8] =
    include_bytes!("../../../crates/spectrum-fonts/tests/fonts/noto-sans-static-source.ttf");

fn actor() -> Actor {
    Actor {
        id: "test:optimized-copy".into(),
        display_name: "Optimized Copy Test".into(),
        kind: ActorKind::Agent,
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

#[test]
fn rewrites_linear_history_with_union_font_repertoire_and_exact_snapshots() {
    let directory = directory("optimized-history");
    let (source, source_hash, revision_count) = project_with_historical_font_usage(&directory);
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
    for revision in &revisions {
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
    }
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
