use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

#[test]
fn colors_accept_rgb_and_rgba() {
    assert_eq!(parse_color("ae7bff").unwrap(), [174, 123, 255, 255]);
    assert_eq!(parse_color("#01020304").unwrap(), [1, 2, 3, 4]);
}

#[test]
fn typography_cli_parses_explicit_face_paragraph_and_effect_controls() {
    let cli = Cli::try_parse_from([
        "prism",
        "--project",
        "type.prism",
        "typography",
        "7",
        "--family",
        "Hack",
        "--weight",
        "700",
        "--style",
        "Bold",
        "--align",
        "right",
        "--line-height",
        "0.8",
        "--tracking",
        "-2",
        "--box-width",
        "420",
        "--outline-width",
        "2",
        "--shadow-x",
        "4",
        "--shadow-y",
        "6",
    ])
    .unwrap();
    let CliCommand::Typography(arguments) = cli.command else {
        panic!("typography subcommand should parse");
    };
    assert_eq!(arguments.id, 7);
    assert_eq!(arguments.family.as_deref(), Some("Hack"));
    assert_eq!(arguments.weight, Some(700));
    assert_eq!(arguments.style.as_deref(), Some("Bold"));
    assert_eq!(arguments.line_height, Some(0.8));
    assert_eq!(arguments.tracking, Some(-2.0));
    assert_eq!(arguments.box_width, Some(420.0));
    assert_eq!(arguments.outline_width, Some(2.0));
    assert_eq!(arguments.shadow_x, Some(4.0));
    assert_eq!(arguments.shadow_y, Some(6.0));
}

#[test]
fn rotate_cli_persists_the_normalized_angle() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let project = std::env::temp_dir().join(format!("prism-rotate-cli-{stamp}.prism"));
    run(Cli {
        project: project.clone(),
        session: None,
        command: CliCommand::Init {
            name: "Rotate CLI".into(),
            width: 400,
            height: 300,
            background: "18191dff".into(),
        },
    })
    .unwrap();
    run(Cli {
        project: project.clone(),
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
