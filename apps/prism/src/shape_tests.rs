use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-{label}-{stamp}"))
}

#[test]
fn editable_shapes_render_fill_and_inside_strokes() {
    let mut workspace = Workspace::new(Document::new("Shapes", 64, 40), None);
    workspace.document.background = [0, 0, 0, 0];
    workspace
        .execute(Command::AddEllipse {
            name: Some("Badge".into()),
            width: 24,
            height: 24,
            color: [220, 40, 60, 255],
            x: 4.0,
            y: 4.0,
        })
        .unwrap();
    let ellipse = workspace.document.selected.unwrap();
    workspace
        .execute(Command::SetShapeStroke {
            id: ellipse,
            stroke: ShapeStroke {
                enabled: true,
                width: 3.0,
                color: [20, 30, 220, 255],
            },
        })
        .unwrap();

    let rendered = render_document(&workspace.document, None)
        .unwrap()
        .to_rgba8();
    assert_eq!(rendered.get_pixel(4, 4)[3], 0, "ellipse corners stay clear");
    assert_eq!(rendered.get_pixel(16, 4).0, [20, 30, 220, 255]);
    assert_eq!(rendered.get_pixel(16, 16).0, [220, 40, 60, 255]);
}

#[test]
fn durable_workspace_round_trips_ellipse_and_stroke() {
    let directory = test_directory("durable-shapes");
    std::fs::create_dir_all(&directory).unwrap();
    let project_path = directory.join("shapes.prism");
    let session = spectrum_revisions::SessionId::new();
    let actor = spectrum_revisions::Actor {
        id: "person:shapes".into(),
        display_name: "Shape tester".into(),
        kind: spectrum_revisions::ActorKind::Human,
    };
    let mut workspace = Workspace::create_durable(
        Document::new("Shapes", 400, 300),
        &project_path,
        actor.clone(),
        session,
    )
    .unwrap();
    workspace
        .execute(Command::AddEllipse {
            name: Some("Orbit".into()),
            width: 180,
            height: 120,
            color: [10, 20, 30, 255],
            x: 40.0,
            y: 60.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetShapeStroke {
            id: 1,
            stroke: ShapeStroke {
                enabled: true,
                width: 7.5,
                color: [240, 230, 220, 255],
            },
        })
        .unwrap();
    drop(workspace);

    let reopened = Workspace::open_as(&project_path, actor, session).unwrap();
    let layer = reopened.document.layer(1).unwrap();
    assert!(matches!(
        layer.kind,
        LayerKind::Ellipse {
            width: 180,
            height: 120,
            ..
        }
    ));
    assert_eq!(layer.stroke.width, 7.5);
    assert_eq!(layer.stroke.color, [240, 230, 220, 255]);
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}
