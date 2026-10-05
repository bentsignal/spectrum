use super::*;
use spectrum_revisions::{Actor, ActorKind, SessionId};

fn actor() -> Actor {
    Actor {
        id: "test:document-lifecycle".into(),
        display_name: "Document lifecycle test".into(),
        kind: ActorKind::System,
    }
}

#[test]
fn document_rename_is_trimmed_bounded_and_undoable() {
    let mut workspace = Workspace::new(Document::new("Original", 320, 200), None);
    workspace
        .execute(Command::RenameDocument {
            name: "  Campaign  ".into(),
        })
        .unwrap();
    assert_eq!(workspace.document.name, "Campaign");
    assert!(workspace.is_dirty());
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(workspace.document.name, "Original");
    workspace.execute(Command::Redo).unwrap();
    assert_eq!(workspace.document.name, "Campaign");

    for invalid in [
        String::new(),
        " \t ".into(),
        "line\nbreak".into(),
        "x".repeat(MAX_DOCUMENT_NAME_CHARS + 1),
    ] {
        assert!(
            workspace
                .execute(Command::RenameDocument { name: invalid })
                .is_err()
        );
        assert_eq!(workspace.document.name, "Campaign");
    }
}

#[test]
fn durable_rename_persists_one_revision_without_changing_the_project_path() {
    let directory =
        std::env::temp_dir().join(format!("prism-document-lifecycle-{}", SessionId::new()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("stable-file-name.prism");
    let session = SessionId::new();
    let mut workspace =
        Workspace::create_durable(Document::new("Original", 320, 200), &path, actor(), session)
            .unwrap();
    let history_before = workspace.history().unwrap().unwrap().revisions.len();

    workspace
        .execute(Command::RenameDocument {
            name: "Renamed metadata".into(),
        })
        .unwrap();
    assert_eq!(workspace.project_path.as_deref(), Some(path.as_path()));
    assert_eq!(
        workspace.history().unwrap().unwrap().revisions.len(),
        history_before + 1
    );
    drop(workspace);

    let reopened = Workspace::open_as(&path, actor(), SessionId::new()).unwrap();
    assert_eq!(reopened.document.name, "Renamed metadata");
    assert_eq!(reopened.project_path.as_deref(), Some(path.as_path()));
    assert!(path.exists());
    assert!(!directory.join("Renamed metadata.prism").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn whole_layer_adjustments_replace_and_undo() {
    let mut workspace = Workspace::new(Document::new("Adjust", 64, 64), None);
    let id = workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 10,
            height: 10,
            color: [200, 120, 80, 255],
            corner_radius: 0.0,
            x: 0.0,
            y: 0.0,
        })
        .unwrap()
        .layer_ids[0];
    let mut adjustments = spectrum_imaging::Adjustments {
        exposure: 0.5,
        ..Default::default()
    };
    adjustments.curves.master.points = vec![
        spectrum_imaging::CurvePoint { x: 0.0, y: 0.0 },
        spectrum_imaging::CurvePoint { x: 0.5, y: 0.7 },
        spectrum_imaging::CurvePoint { x: 1.0, y: 1.0 },
    ];
    adjustments.hsl.red.saturation = 30.0;
    workspace
        .execute(Command::SetLayerAdjustments {
            id,
            adjustments: adjustments.clone(),
        })
        .unwrap();
    let stored = &workspace.document.layer(id).unwrap().adjustments;
    assert_eq!(stored.curves.master.points.len(), 3);
    assert_eq!(stored.hsl.red.saturation, 30.0);
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(
        workspace.document.layer(id).unwrap().adjustments,
        spectrum_imaging::Adjustments::default()
    );
    assert!(
        workspace
            .execute(Command::SetLayerAdjustments {
                id: 999,
                adjustments,
            })
            .is_err()
    );
}
