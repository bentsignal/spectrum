use crate::{Command, Document, LayerKind, TextTypography, Workspace};
use std::time::{SystemTime, UNIX_EPOCH};

fn durable_workspace(label: &str) -> (Workspace, std::path::PathBuf) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("prism-{label}-{stamp}"));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("paragraph.prism");
    let workspace = Workspace::create_durable(
        Document::new("Paragraph", 400, 300),
        &project,
        spectrum_revisions::Actor {
            id: "test:paragraph-drag".into(),
            display_name: "Paragraph drag test".into(),
            kind: spectrum_revisions::ActorKind::Human,
        },
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    (workspace, project)
}

#[test]
fn preview_batch_commits_paragraph_text_as_one_undoable_interaction() {
    let (mut workspace, project) = durable_workspace("paragraph-batch");
    assert_eq!(workspace.history().unwrap().unwrap().revisions.len(), 1);
    workspace.begin_interaction();
    workspace
        .preview_batch(vec![
            Command::AddText {
                text: "Wrapped words".into(),
                name: None,
                font_size: 32.0,
                color: [255, 255, 255, 255],
                x: 40.0,
                y: 60.0,
            },
            Command::SetTextTypography {
                id: 1,
                typography: TextTypography {
                    box_width: Some(180.0),
                    ..Default::default()
                },
            },
        ])
        .unwrap();
    assert!(workspace.commit_interaction().unwrap());
    let LayerKind::Text { typography, .. } = &workspace.document.layer(1).unwrap().kind else {
        panic!("created layer should be text");
    };
    assert_eq!(typography.box_width, Some(180.0));
    let history = workspace.history().unwrap().unwrap();
    assert_eq!(history.revisions.len(), 2);
    assert_eq!(
        history
            .revisions
            .iter()
            .find(|revision| revision.id == history.current)
            .unwrap()
            .command_count,
        2
    );

    drop(workspace);
    let mut workspace = Workspace::open(&project).unwrap();
    let LayerKind::Text { typography, .. } = &workspace.document.layer(1).unwrap().kind else {
        panic!("replayed layer should be text");
    };
    assert_eq!(typography.box_width, Some(180.0));

    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.layers.is_empty());
    assert!(workspace.execute(Command::Undo).is_err());
    drop(workspace);
    std::fs::remove_dir_all(project.parent().unwrap()).unwrap();
}

#[test]
fn failed_preview_batch_is_atomic_and_cancelable() {
    let mut workspace = Workspace::new(Document::new("Paragraph", 400, 300), None);
    let before = workspace.document.clone();
    workspace.begin_interaction();
    assert!(
        workspace
            .preview_batch(vec![
                Command::AddText {
                    text: "Draft".into(),
                    name: None,
                    font_size: 32.0,
                    color: [255, 255, 255, 255],
                    x: 40.0,
                    y: 60.0,
                },
                Command::SetTextTypography {
                    id: 99,
                    typography: TextTypography {
                        box_width: Some(180.0),
                        ..Default::default()
                    },
                },
            ])
            .is_err()
    );
    assert_eq!(workspace.document, before);
    assert!(workspace.cancel_interaction());
    assert!(!workspace.can_undo());
}
