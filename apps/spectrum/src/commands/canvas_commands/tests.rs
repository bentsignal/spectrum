use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn colors_accept_rgb_and_rgba() {
    assert_eq!(parse_color("ae7bff").unwrap(), [174, 123, 255, 255]);
    assert_eq!(parse_color("#01020304").unwrap(), [1, 2, 3, 4]);
}

#[test]
fn path_and_vector_mask_cli_surfaces_mutate_durable_projects_end_to_end() {
    let project = temporary_project("path-vector-mask");
    let open_path = project.with_extension("open-path.json");
    let closed_path = project.with_extension("closed-path.json");
    let open = spectrum_canvas::PathGeometry::new(
        80,
        60,
        false,
        spectrum_canvas::PathFillRule::EvenOdd,
        vec![
            spectrum_canvas::PathAnchor::corner(2.0, 55.0),
            spectrum_canvas::PathAnchor {
                point: [40.0, 3.0],
                handle_in: [-18.0, 0.0],
                handle_out: [18.0, 0.0],
            },
            spectrum_canvas::PathAnchor::corner(78.0, 55.0),
        ],
    )
    .unwrap();
    let closed = spectrum_canvas::PathGeometry::new(
        100,
        100,
        true,
        spectrum_canvas::PathFillRule::EvenOdd,
        vec![
            spectrum_canvas::PathAnchor::corner(50.0, 0.0),
            spectrum_canvas::PathAnchor::corner(100.0, 100.0),
            spectrum_canvas::PathAnchor::corner(0.0, 100.0),
        ],
    )
    .unwrap();
    std::fs::write(&open_path, serde_json::to_vec(&open).unwrap()).unwrap();
    std::fs::write(&closed_path, serde_json::to_vec(&closed).unwrap()).unwrap();
    let project_arg = project.to_str().unwrap();
    let open_arg = open_path.to_str().unwrap();
    let closed_arg = closed_path.to_str().unwrap();
    for arguments in [
        vec!["init", "Path CLI", "--width", "240", "--height", "180"],
        vec![
            "path",
            "add",
            open_arg,
            "--name",
            "CLI Curve",
            "--color",
            "aabbccdd",
            "--x",
            "12",
            "--y",
            "18",
        ],
        vec!["path", "replace", "1", closed_arg],
        vec!["add-rectangle", "--width", "120", "--height", "60"],
        vec!["vector-mask", "2", closed_arg, "--invert"],
    ] {
        let mut cli = vec!["canvas", "--document", project_arg];
        cli.extend(arguments);
        run(parse_cli(cli).unwrap()).unwrap();
    }
    let document = Workspace::read(&project).unwrap();
    let spectrum_canvas::LayerKind::Path { geometry, color } = &document.layer(1).unwrap().kind
    else {
        panic!("path CLI did not create a path layer")
    };
    assert_eq!(geometry, &closed);
    assert_eq!(*color, [0xaa, 0xbb, 0xcc, 0xdd]);
    assert!(
        document
            .layer(2)
            .unwrap()
            .vector_mask
            .as_ref()
            .unwrap()
            .invert
    );

    run(parse_cli([
        "canvas",
        "--document",
        project_arg,
        "vector-mask",
        "2",
        "--clear",
    ])
    .unwrap())
    .unwrap();
    assert!(
        Workspace::read(&project)
            .unwrap()
            .layer(2)
            .unwrap()
            .vector_mask
            .is_none()
    );
    for path in [project, open_path, closed_path] {
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn selection_cli_persists_and_fills_without_touching_existing_layers() {
    let project = temporary_project("selection");
    let project_arg = project.to_str().unwrap();
    for arguments in [
        vec!["init", "Selection CLI", "--width", "80", "--height", "60"],
        vec![
            "add-rectangle",
            "--width",
            "12",
            "--height",
            "10",
            "--x",
            "30",
            "--y",
            "20",
        ],
        vec!["selection", "rectangle", "4", "5", "20", "10"],
        vec!["selection", "fill", "--color", "12345678", "--name", "Wash"],
    ] {
        let mut cli = vec!["canvas", "--document", project_arg];
        cli.extend(arguments);
        run(parse_cli(cli).unwrap()).unwrap();
    }
    let document = Workspace::read(&project).unwrap();
    assert_eq!(
        document.selection,
        Some(spectrum_canvas::Selection::rectangle(4, 5, 20, 10))
    );
    assert_eq!(document.layers.len(), 2);
    assert_eq!(document.layers[0].name, "Rectangle");
    assert_eq!(document.layers[1].name, "Wash");
    assert_eq!(
        (
            document.layers[1].transform.x,
            document.layers[1].transform.y
        ),
        (4.0, 5.0)
    );
    assert!(matches!(
        document.layers[1].kind,
        spectrum_canvas::LayerKind::Rectangle {
            width: 20,
            height: 10,
            color: [0x12, 0x34, 0x56, 0x78],
            corner_radius: 0.0,
        }
    ));
    std::fs::remove_file(project).unwrap();
}

#[test]
fn selection_crop_cli_uses_the_atomic_core_command() {
    let project = temporary_project("selection-crop");
    let project_arg = project.to_str().unwrap();
    for arguments in [
        vec!["init", "Selection crop", "--width", "80", "--height", "60"],
        vec![
            "add-rectangle",
            "--width",
            "12",
            "--height",
            "10",
            "--x",
            "30",
            "--y",
            "20",
        ],
        vec!["selection", "rectangle", "4", "5", "20", "10"],
        vec!["selection", "crop"],
    ] {
        let mut cli = vec!["canvas", "--document", project_arg];
        cli.extend(arguments);
        run(parse_cli(cli).unwrap()).unwrap();
    }
    let document = Workspace::read(&project).unwrap();
    assert_eq!((document.width, document.height), (20, 10));
    assert_eq!(document.selection, None);
    assert_eq!(document.layers.len(), 1);
    assert_eq!(
        (
            document.layers[0].transform.x,
            document.layers[0].transform.y,
        ),
        (26.0, 15.0)
    );
    std::fs::remove_file(project).unwrap();
}

#[test]
fn benchmark_cli_defaults_to_interactive_and_accepts_hosted_ci() {
    let default = parse_cli(["canvas", "benchmark", "--strict"]).unwrap();
    let CliCommand::Benchmark {
        strict,
        profile: default_profile,
    } = default.command
    else {
        panic!("benchmark subcommand should parse");
    };
    assert!(strict);
    assert_eq!(default_profile.name(), "interactive-workstation");
    assert_eq!(default_profile.gradient_shadow_budget_ms(), 500.0);
    assert_eq!(default_profile.magic_wand_budget_ms(), 5_000.0);

    let hosted = parse_cli(["canvas", "benchmark", "--strict", "--profile", "hosted-ci"]).unwrap();
    let CliCommand::Benchmark {
        profile: hosted_profile,
        ..
    } = hosted.command
    else {
        panic!("benchmark subcommand should parse");
    };
    assert_eq!(hosted_profile.name(), "github-hosted-linux");
    assert_eq!(hosted_profile.gradient_shadow_budget_ms(), 1_250.0);
    assert_eq!(hosted_profile.magic_wand_budget_ms(), 15_000.0);
    assert!(222.508 <= default_profile.gradient_shadow_budget_ms());
    assert!(880.788 <= hosted_profile.gradient_shadow_budget_ms());
    assert!(hosted_profile.gradient_shadow_budget_ms() < 2_061.886);
}

#[test]
fn typography_cli_parses_face_paragraph_and_effect_controls() {
    let cli = parse_cli([
        "canvas",
        "--document",
        "type.spectrum",
        "typography",
        "7",
        "--family",
        "Hack",
        "--weight",
        "700",
        "--style",
        "Bold",
        "--layout",
        "harfbuzz-v1",
        "--language",
        "iw-IL",
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
    assert_eq!(
        arguments.layout,
        Some(typography::CliTextLayout::HarfbuzzV1)
    );
    assert_eq!(arguments.language.as_deref(), Some("iw-IL"));
    assert_eq!(arguments.line_height, Some(0.8));
    assert_eq!(arguments.tracking, Some(-2.0));
    assert_eq!(arguments.box_width, Some(420.0));
    assert_eq!(arguments.outline_width, Some(2.0));
    assert_eq!(arguments.shadow_x, Some(4.0));
    assert_eq!(arguments.shadow_y, Some(6.0));
}

#[test]
fn add_text_defaults_to_shaped_layout_and_typography_can_explicitly_upgrade_or_downgrade() {
    let project = temporary_project("shaped-text-cli");
    let project_arg = project.to_str().unwrap();
    for arguments in [
        vec!["init", "Shaped CLI", "--width", "320", "--height", "180"],
        vec!["add-text", "office العربية", "--language", "iw-IL"],
    ] {
        let mut cli = vec!["canvas", "--document", project_arg];
        cli.extend(arguments);
        run(parse_cli(cli).unwrap()).unwrap();
    }
    let document = Workspace::read(&project).unwrap();
    let spectrum_canvas::LayerKind::Text { typography, .. } = &document.layer(1).unwrap().kind
    else {
        panic!("CLI did not create text");
    };
    assert_eq!(
        typography.shaping.engine,
        spectrum_canvas::TextShapingEngine::HarfBuzzV1
    );
    assert_eq!(typography.shaping.language.as_deref(), Some("he-IL"));

    run(parse_cli([
        "canvas",
        "--document",
        project_arg,
        "typography",
        "1",
        "--layout",
        "legacy-v1",
    ])
    .unwrap())
    .unwrap();
    let document = Workspace::read(&project).unwrap();
    let spectrum_canvas::LayerKind::Text { typography, .. } = &document.layer(1).unwrap().kind
    else {
        panic!("CLI text disappeared");
    };
    assert_eq!(
        typography.shaping.engine,
        spectrum_canvas::TextShapingEngine::LegacyCharV1
    );
    assert_eq!(typography.shaping.language, None);
    std::fs::remove_file(project).unwrap();
}

#[test]
fn font_list_cli_accepts_an_optional_query() {
    let cli = parse_cli([
        "canvas",
        "--document",
        "type.spectrum",
        "font-list",
        "--query",
        "hack",
    ])
    .unwrap();
    let CliCommand::FontList { query, .. } = cli.command else {
        panic!("font-list subcommand should parse");
    };
    assert_eq!(query.as_deref(), Some("hack"));
}

#[test]
fn bundled_font_output_is_truthful_and_legacy_family_automation_remains_compatible() {
    let mut workspace = Workspace::new(Document::new("Bundled font", 320, 200));
    workspace
        .execute(Command::AddText {
            text: "Legacy automation".into(),
            name: None,
            font_size: 32.0,
            color: [255; 4],
            x: 0.0,
            y: 0.0,
            shaping: Default::default(),
        })
        .unwrap();
    let output = typography::font_list(&workspace.document, None, false);
    // Installed fonts are listed only when asked for.
    assert!(output.get("system").is_none());
    let with_system = typography::font_list(&workspace.document, None, true);
    assert!(with_system["system"].is_array());
    assert_eq!(output["bundled"]["id"], serde_json::Value::Null);
    assert_eq!(output["bundled"]["family"], "Ubuntu");
    assert_eq!(output["bundled"]["style"], "Light");
    assert_eq!(output["bundled"]["designed_by"], "Dalton Maag");
    assert_eq!(output["bundled"]["license_name"], "Ubuntu Font Licence 1.0");
    assert_eq!(
        output["bundled"]["compatibility_aliases"][0],
        "Spectrum Sans"
    );
    assert_ne!(output["bundled"]["family"], "Spectrum Sans");

    let cli = parse_cli([
        "canvas",
        "--document",
        "type.spectrum",
        "typography",
        "1",
        "--family",
        "Spectrum Sans",
    ])
    .unwrap();
    let CliCommand::Typography(arguments) = cli.command else {
        panic!("typography subcommand should parse");
    };
    let updated = typography::updated_typography(&workspace.document, &arguments).unwrap();
    assert_eq!(updated.font_id, None);
}

#[test]
fn font_usage_cli_accepts_an_optional_asset_filter() {
    let cli = parse_cli([
        "canvas",
        "--document",
        "type.spectrum",
        "font-usage",
        "--font-id",
        "12",
    ])
    .unwrap();
    let CliCommand::FontUsage { font_id } = cli.command else {
        panic!("font-usage subcommand should parse");
    };
    assert_eq!(font_id, Some(12));
}

#[test]
fn font_usage_output_limits_its_non_mutation_and_coverage_claims() {
    let output = typography::font_usage(&Document::new("Usage", 320, 200), None).unwrap();
    assert_eq!(output["action"], "font_usage");
    assert_eq!(output["analysis_scope"], "unicode_cmap_subset_retention");
    assert_eq!(output["font_bytes_modified"], false);
    assert!(output.get("mutates_project").is_none());
    assert_eq!(output["editable_font_bytes_preserved"], true);
    assert_eq!(output["limitations"].as_array().unwrap().len(), 2);
    assert_eq!(output["fonts"], serde_json::json!([]));
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

    let copy = parse_cli([
        "canvas",
        "--document",
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

    let paste = parse_cli([
        "canvas",
        "--document",
        destination.to_str().unwrap(),
        "layer-paste",
        transfer.to_str().unwrap(),
        "--index",
        "0",
    ])
    .unwrap();
    run(paste).unwrap();
    let workspace = Workspace::open(&destination, cli_actor(), SessionId::new()).unwrap();
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
    let cli = parse_cli([
        "canvas",
        "--document",
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
    let rotate = parse_cli([
        "canvas",
        "--document",
        project.to_str().unwrap(),
        "rotate",
        "1",
        "-15",
    ])
    .unwrap();
    run(rotate).unwrap();
    let document = Workspace::read(&project).unwrap();
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
        let mut cli = vec!["canvas", "--document", project.to_str().unwrap()];
        cli.extend(arguments);
        run(parse_cli(cli).unwrap()).unwrap();
    }
    let document = Workspace::read(&project).unwrap();
    assert!(!document.snapping_enabled);
    assert_eq!(document.guides[0].position, 125.5);
    let geometry = spectrum_canvas::layer_geometry(document.layer(1).unwrap()).unwrap();
    assert!((geometry.center[0] - 200.0).abs() < 0.001);
    std::fs::remove_file(project).unwrap();
}

pub(super) fn temporary_project(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!("canvas-{label}-cli-{stamp}.spectrum"))
}

#[path = "tests_support.rs"]
mod support;
use support::initialize_rectangle_project;
