use super::*;
use image::{Rgba, RgbaImage};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn test_directory(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn adjustment_history_survives_navigation() {
    let mut project = Project::new("test");
    project.photos.push(crate::Photo::new(
        1,
        "test.jpg".into(),
        "test.jpg".into(),
        1,
        1,
    ));
    let mut workspace = Workspace::new(project, None);
    workspace
        .execute(Command::Adjust {
            id: 1,
            patch: AdjustmentPatch {
                exposure: Some(2.0),
                ..Default::default()
            },
        })
        .unwrap();
    assert_eq!(
        workspace.project.photo(1).unwrap().adjustments.exposure,
        2.0
    );
    workspace.execute(Command::HistoryBack { id: 1 }).unwrap();
    assert_eq!(
        workspace.project.photo(1).unwrap().adjustments.exposure,
        0.0
    );
    workspace
        .execute(Command::HistoryForward { id: 1 })
        .unwrap();
    assert_eq!(
        workspace.project.photo(1).unwrap().adjustments.exposure,
        2.0
    );
}

#[test]
fn failed_multi_import_is_transactional() {
    let directory = test_directory("lumen-import");
    fs::create_dir_all(&directory).unwrap();
    let valid = directory.join("valid.png");
    let invalid = directory.join("invalid.jpg");
    RgbaImage::from_pixel(2, 2, Rgba([20, 40, 60, 255]))
        .save(&valid)
        .unwrap();
    fs::write(&invalid, b"not an image").unwrap();
    let mut workspace = Workspace::default();
    assert!(
        workspace
            .execute(Command::Import {
                paths: vec![valid, invalid]
            })
            .is_err()
    );
    assert!(workspace.project.photos.is_empty());
    assert!(workspace.project.batches.is_empty());
    assert!(!workspace.can_undo());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn successful_import_creates_a_shoot_batch() {
    let directory = test_directory("lumen-batch-import");
    fs::create_dir_all(&directory).unwrap();
    let first = directory.join("one.png");
    let second = directory.join("two.png");
    RgbaImage::from_pixel(2, 2, Rgba([20, 40, 60, 255]))
        .save(&first)
        .unwrap();
    RgbaImage::from_pixel(2, 2, Rgba([60, 40, 20, 255]))
        .save(&second)
        .unwrap();
    let mut workspace = Workspace::default();
    workspace
        .execute(Command::Import {
            paths: vec![first, second],
        })
        .unwrap();
    assert_eq!(workspace.project.batches.len(), 1);
    let batch_id = workspace.project.batches[0].id;
    assert!(
        workspace
            .project
            .photos
            .iter()
            .all(|photo| photo.batch_id == Some(batch_id))
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn presets_apply_to_multiple_photos_without_geometry() {
    let mut project = Project::new("test");
    let mut first = crate::Photo::new(1, "one.jpg".into(), "one.jpg".into(), 1, 1);
    first.adjustments.exposure = 1.0;
    let mut second = crate::Photo::new(2, "two.jpg".into(), "two.jpg".into(), 1, 1);
    second.adjustments.rotation = 90;
    project.photos.extend([first, second]);
    let mut workspace = Workspace::new(project, None);
    workspace
        .execute(Command::SavePreset {
            name: "Bright".into(),
            from_id: 1,
        })
        .unwrap();
    workspace
        .execute(Command::ApplyPreset {
            preset_id: 1,
            ids: vec![2],
        })
        .unwrap();
    let second = workspace.project.photo(2).unwrap();
    assert_eq!(second.adjustments.exposure, 1.0);
    assert_eq!(second.adjustments.rotation, 90);
    assert_eq!(second.history.last().unwrap().label, "Preset: Bright");
}

#[test]
fn pick_state_is_a_core_catalog_command() {
    let mut project = Project::new("test");
    project.photos.push(crate::Photo::new(
        1,
        "one.jpg".into(),
        "one.jpg".into(),
        1,
        1,
    ));
    let mut workspace = Workspace::new(project, None);
    workspace
        .execute(Command::SetPick {
            ids: vec![1],
            state: PickState::Keep,
        })
        .unwrap();
    assert_eq!(workspace.project.photo(1).unwrap().pick, PickState::Keep);
    assert!(workspace.can_undo());
}

#[test]
fn batches_can_be_renamed_and_are_pruned_when_empty() {
    let mut project = Project::new("test");
    let mut photo = crate::Photo::new(1, "one.jpg".into(), "one.jpg".into(), 1, 1);
    photo.batch_id = Some(1);
    project.photos.push(photo);
    project.batches.push(crate::PhotoBatch {
        id: 1,
        name: "Shoot 1".into(),
        captured_date: None,
        captured_end_date: None,
        imported_date: "2026-07-16".into(),
    });
    let mut workspace = Workspace::new(project, None);
    workspace
        .execute(Command::RenameBatch {
            id: 1,
            name: "Night walk".into(),
        })
        .unwrap();
    assert_eq!(workspace.project.batch(1).unwrap().name, "Night walk");
    workspace.execute(Command::Remove { ids: vec![1] }).unwrap();
    assert!(workspace.project.batches.is_empty());
}
