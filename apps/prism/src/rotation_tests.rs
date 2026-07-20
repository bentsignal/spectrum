use super::*;

fn rectangle_workspace() -> (Workspace, u64) {
    let mut workspace = Workspace::new(Document::new("Rotate", 400, 300), None);
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 100,
            height: 80,
            color: [255, 255, 255, 255],
            corner_radius: 0.0,
            x: 10.0,
            y: 20.0,
        })
        .unwrap();
    let id = workspace.document.selected.unwrap();
    (workspace, id)
}

#[test]
fn rotation_command_normalizes_degrees_and_respects_locking() {
    let (mut workspace, id) = rectangle_workspace();
    workspace
        .execute(Command::SetRotation { id, degrees: -15.0 })
        .unwrap();
    assert_eq!(
        workspace.document.layer(id).unwrap().transform.rotation,
        345.0
    );

    workspace
        .execute(Command::SetLocked { id, locked: true })
        .unwrap();
    let error = workspace
        .execute(Command::SetRotation { id, degrees: 90.0 })
        .unwrap_err();
    assert!(error.to_string().contains("locked"));
}

#[test]
fn rotation_previews_coalesce_into_one_undo_step() {
    let (mut workspace, id) = rectangle_workspace();
    workspace.begin_interaction();
    for degrees in 1..=73 {
        workspace
            .preview(Command::SetRotation {
                id,
                degrees: degrees as f32,
            })
            .unwrap();
    }
    assert!(workspace.commit_interaction().unwrap());
    assert_eq!(
        workspace.document.layer(id).unwrap().transform.rotation,
        73.0
    );
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(
        workspace.document.layer(id).unwrap().transform.rotation,
        0.0
    );
    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.layers.is_empty());
}

#[test]
fn canceled_rotation_restores_the_angle_without_adding_history() {
    let (mut workspace, id) = rectangle_workspace();
    workspace.begin_interaction();
    workspace
        .preview(Command::SetRotation { id, degrees: 73.0 })
        .unwrap();
    assert!(workspace.cancel_interaction());
    assert_eq!(
        workspace.document.layer(id).unwrap().transform.rotation,
        0.0
    );
    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.layers.is_empty());
}

#[test]
fn empty_rotation_interaction_does_not_add_history() {
    let (mut workspace, _) = rectangle_workspace();
    workspace.begin_interaction();
    assert!(!workspace.commit_interaction().unwrap());
    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.layers.is_empty());
}
