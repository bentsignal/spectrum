use super::*;

fn rectangle_workspace() -> (Workspace, u64) {
    let mut workspace = Workspace::new(Document::new("Rotate", 400, 300));
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
