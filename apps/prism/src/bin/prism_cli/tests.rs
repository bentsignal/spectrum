use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn colors_accept_rgb_and_rgba() {
    assert_eq!(parse_color("ae7bff").unwrap(), [174, 123, 255, 255]);
    assert_eq!(parse_color("#01020304").unwrap(), [1, 2, 3, 4]);
}

#[test]
fn rotate_cli_persists_the_normalized_angle() {
    let project = temporary_project("rotate");
    initialize_rectangle_project(&project);
    let rotate = Cli::try_parse_from([
        "prism",
        "--project",
        project.to_str().unwrap(),
        "rotate",
        "1",
        "-15",
    ])
    .unwrap();
    run(rotate).unwrap();
    let document = Workspace::load_read_only(&project).unwrap();
    assert_eq!(document.layer(1).unwrap().transform.rotation, 345.0);
    std::fs::remove_file(project).unwrap();
}

#[test]
fn guide_snapping_and_alignment_cli_persist_semantic_commands() {
    let project = temporary_project("alignment");
    initialize_rectangle_project(&project);
    for arguments in [
        vec!["snapping", "false"],
        vec!["guide", "add", "vertical", "125.5"],
        vec!["align", "1", "horizontal-center"],
    ] {
        let mut cli = vec!["prism", "--project", project.to_str().unwrap()];
        cli.extend(arguments);
        run(Cli::try_parse_from(cli).unwrap()).unwrap();
    }
    let document = Workspace::load_read_only(&project).unwrap();
    assert!(!document.snapping_enabled);
    assert_eq!(document.guides[0].position, 125.5);
    let geometry = prism_core::layer_geometry(document.layer(1).unwrap()).unwrap();
    assert!((geometry.center[0] - 200.0).abs() < 0.001);
    std::fs::remove_file(project).unwrap();
}

fn temporary_project(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-{label}-cli-{stamp}.prism"))
}

fn initialize_rectangle_project(project: &Path) {
    run(Cli {
        project: project.to_owned(),
        session: None,
        command: CliCommand::Init {
            name: "CLI test".into(),
            width: 400,
            height: 300,
            background: "18191dff".into(),
        },
    })
    .unwrap();
    run(Cli {
        project: project.to_owned(),
        session: None,
        command: CliCommand::AddRectangle {
            name: None,
            width: 100,
            height: 80,
            color: "ffffffff".into(),
            radius: 0.0,
            x: 10.0,
            y: 20.0,
        },
    })
    .unwrap();
}
