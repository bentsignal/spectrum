use std::{path::Path, process::Command as ProcessCommand, str::FromStr};

use prism_core::{Command, Document, Workspace};
use serde_json::Value;
use spectrum_revisions::{
    Actor, ActorKind, CollaborationMode, CollaborationStatus, CollaborationSync, SessionId,
};

#[test]
fn cli_agent_sessions_support_together_and_separate_workflows() {
    let human_session = SessionId::new();
    let directory = std::env::temp_dir().join(format!("prism-cli-agent-{human_session}"));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("cli-agent.prism");
    let mut human = Workspace::create_durable(
        Document::new("CLI collaboration", 640, 480),
        &project,
        Actor {
            id: "person:cli-test".into(),
            display_name: "CLI Test User".into(),
            kind: ActorKind::Human,
        },
        human_session,
    )
    .unwrap();

    let together = run_prism(&[
        "--project",
        project.to_str().unwrap(),
        "agent",
        "start",
        "--mode",
        "together",
        "--name",
        "Codex Test",
    ]);
    assert_eq!(together["mode"], "together");
    let together_session = session(&together);
    run_prism(&[
        "--project",
        project.to_str().unwrap(),
        "--session",
        &together_session.to_string(),
        "add-text",
        "Together result",
        "--name",
        "Together result",
    ]);
    assert!(matches!(
        human.sync_together().unwrap(),
        CollaborationSync::Advanced { .. }
    ));
    assert_eq!(human.document.layers[0].name, "Together result");

    human
        .execute(Command::AddRectangle {
            name: Some("Human result".into()),
            width: 120,
            height: 80,
            color: [40, 180, 120, 255],
            corner_radius: 12.0,
            x: 50.0,
            y: 100.0,
        })
        .unwrap();
    run_prism(&[
        "--project",
        project.to_str().unwrap(),
        "--session",
        &together_session.to_string(),
        "add-text",
        "Agent alternate",
    ]);
    assert!(matches!(
        human.sync_together().unwrap(),
        CollaborationSync::Split(_)
    ));
    let together_status = status(&project, together_session);
    assert_eq!(
        together_status["collaboration"]["status"],
        serde_json::to_value(CollaborationStatus::Split).unwrap()
    );

    let separate = run_prism(&[
        "--project",
        project.to_str().unwrap(),
        "agent",
        "start",
        "--mode",
        "separate",
        "--name",
        "Claude Test",
    ]);
    assert_eq!(
        separate["mode"],
        serde_json::to_value(CollaborationMode::Separate).unwrap()
    );
    let separate_session = session(&separate);
    run_prism(&[
        "--project",
        project.to_str().unwrap(),
        "--session",
        &separate_session.to_string(),
        "add-text",
        "Separate result",
    ]);
    assert_eq!(human.sync_together().unwrap(), CollaborationSync::Idle);
    assert!(
        human
            .document
            .layers
            .iter()
            .all(|layer| layer.name != "Separate result")
    );
    drop(human);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cli_creates_and_styles_editable_ellipses() {
    let directory = std::env::temp_dir().join(format!(
        "prism-cli-ellipse-{}",
        spectrum_revisions::RevisionId::new()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("ellipse.prism");
    run_prism(&[
        "--project",
        project.to_str().unwrap(),
        "init",
        "Ellipse CLI",
        "--width",
        "640",
        "--height",
        "480",
    ]);
    run_prism(&[
        "--project",
        project.to_str().unwrap(),
        "add-ellipse",
        "--name",
        "Sun",
        "--width",
        "180",
        "--height",
        "120",
        "--color",
        "f7b266ff",
        "--x",
        "40",
        "--y",
        "50",
    ]);
    run_prism(&[
        "--project",
        project.to_str().unwrap(),
        "stroke",
        "1",
        "--width",
        "8",
        "--color",
        "ffffffff",
    ]);
    let listed = run_prism(&["--project", project.to_str().unwrap(), "list"]);
    let layer = &listed["document"]["layers"][0];
    assert_eq!(layer["kind"]["type"], "ellipse");
    assert_eq!(layer["stroke"]["enabled"], true);
    assert_eq!(layer["stroke"]["width"], 8.0);
    std::fs::remove_dir_all(directory).unwrap();
}

fn status(project: &Path, session: SessionId) -> Value {
    run_prism(&[
        "--project",
        project.to_str().unwrap(),
        "--session",
        &session.to_string(),
        "agent",
        "status",
    ])
}

fn session(output: &Value) -> SessionId {
    SessionId::from_str(output["session"].as_str().unwrap()).unwrap()
}

fn run_prism(arguments: &[&str]) -> Value {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_prism"))
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "prism failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
