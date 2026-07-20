use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn colors_accept_rgb_and_rgba() {
    assert_eq!(parse_color("ae7bff").unwrap(), [174, 123, 255, 255]);
    assert_eq!(parse_color("#01020304").unwrap(), [1, 2, 3, 4]);
}

#[test]
fn typography_cli_parses_face_paragraph_and_effect_controls() {
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
fn font_list_cli_accepts_an_optional_query() {
    let cli = Cli::try_parse_from([
        "prism",
        "--project",
        "type.prism",
        "font-list",
        "--query",
        "hack",
    ])
    .unwrap();
    let CliCommand::FontList { query } = cli.command else {
        panic!("font-list subcommand should parse");
    };
    assert_eq!(query.as_deref(), Some("hack"));
}

#[test]
fn schema_keeps_guides_and_typography_commands_together() {
    let schema = schema();
    let examples = schema["command_protocol"]["examples"].as_array().unwrap();
    for command in [
        "align_layer",
        "add_guide",
        "import_font",
        "set_text_typography",
        "insert_layer",
    ] {
        assert!(examples.iter().any(|example| example["command"] == command));
    }
    assert!(schema["alignment"].is_object());
    assert!(schema["typography"].is_object());
    assert_eq!(schema["layer_transfer"]["version"], 1);
    let insert = examples
        .iter()
        .find(|example| example["command"] == "insert_layer")
        .unwrap();
    assert!(matches!(
        serde_json::from_value::<Command>(insert.clone()).unwrap(),
        Command::InsertLayer { .. }
    ));
}

#[test]
fn layer_copy_defaults_to_selection_and_layer_paste_is_one_revision() {
    let source = temporary_project("transfer-source");
    let destination = temporary_project("transfer-destination");
    let transfer = temporary_project("transfer-json").with_extension("json");
    initialize_rectangle_project(&source);
    run(Cli {
        project: destination.clone(),
        session: None,
        command: CliCommand::Init {
            name: "Destination".into(),
            width: 400,
            height: 300,
            background: "18191dff".into(),
        },
    })
    .unwrap();

    let copy = Cli::try_parse_from([
        "prism",
        "--project",
        source.to_str().unwrap(),
        "layer-copy",
        "--output",
        transfer.to_str().unwrap(),
    ])
    .unwrap();
    let copied = run(copy).unwrap();
    assert_eq!(copied["action"], "layer_copy");
    assert_eq!(copied["version"], 1);
    assert!(transfer.exists());

    let paste = Cli::try_parse_from([
        "prism",
        "--project",
        destination.to_str().unwrap(),
        "layer-paste",
        transfer.to_str().unwrap(),
        "--index",
        "0",
    ])
    .unwrap();
    run(paste).unwrap();
    let workspace = Workspace::open(&destination).unwrap();
    assert_eq!(workspace.document.layers.len(), 1);
    assert_eq!(workspace.document.layers[0].name, "Rectangle");
    assert_eq!(workspace.document.selected, Some(1));
    assert_eq!(workspace.history().unwrap().unwrap().revisions.len(), 2);
    drop(workspace);

    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(destination).unwrap();
    std::fs::remove_file(transfer).unwrap();
}

#[test]
fn layer_copy_refuses_to_overwrite_an_existing_transfer_file() {
    let source = temporary_project("transfer-overwrite-source");
    let transfer = temporary_project("transfer-overwrite-json").with_extension("json");
    initialize_rectangle_project(&source);
    std::fs::write(&transfer, "keep me").unwrap();
    let cli = Cli::try_parse_from([
        "prism",
        "--project",
        source.to_str().unwrap(),
        "layer-copy",
        "1",
        "--output",
        transfer.to_str().unwrap(),
    ])
    .unwrap();
    assert!(run(cli).is_err());
    assert_eq!(std::fs::read_to_string(&transfer).unwrap(), "keep me");
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(transfer).unwrap();
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
