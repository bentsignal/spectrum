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
    let open = prism_core::PathGeometry::new(
        80,
        60,
        false,
        prism_core::PathFillRule::EvenOdd,
        vec![
            prism_core::PathAnchor::corner(2.0, 55.0),
            prism_core::PathAnchor {
                point: [40.0, 3.0],
                handle_in: [-18.0, 0.0],
                handle_out: [18.0, 0.0],
            },
            prism_core::PathAnchor::corner(78.0, 55.0),
        ],
    )
    .unwrap();
    let closed = prism_core::PathGeometry::new(
        100,
        100,
        true,
        prism_core::PathFillRule::EvenOdd,
        vec![
            prism_core::PathAnchor::corner(50.0, 0.0),
            prism_core::PathAnchor::corner(100.0, 100.0),
            prism_core::PathAnchor::corner(0.0, 100.0),
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
        let mut cli = vec!["prism", "--project", project_arg];
        cli.extend(arguments);
        run(Cli::try_parse_from(cli).unwrap()).unwrap();
    }
    let document = Workspace::load_read_only(&project).unwrap();
    let prism_core::LayerKind::Path { geometry, color } = &document.layer(1).unwrap().kind else {
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

    run(Cli::try_parse_from([
        "prism",
        "--project",
        project_arg,
        "vector-mask",
        "2",
        "--clear",
    ])
    .unwrap())
    .unwrap();
    assert!(
        Workspace::load_read_only(&project)
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
        let mut cli = vec!["prism", "--project", project_arg];
        cli.extend(arguments);
        run(Cli::try_parse_from(cli).unwrap()).unwrap();
    }
    let document = Workspace::load_read_only(&project).unwrap();
    assert_eq!(
        document.selection,
        Some(prism_core::Selection::rectangle(4, 5, 20, 10))
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
        prism_core::LayerKind::Rectangle {
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
        let mut cli = vec!["prism", "--project", project_arg];
        cli.extend(arguments);
        run(Cli::try_parse_from(cli).unwrap()).unwrap();
    }
    let document = Workspace::load_read_only(&project).unwrap();
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
    let default = Cli::try_parse_from(["prism", "benchmark", "--strict"]).unwrap();
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

    let hosted =
        Cli::try_parse_from(["prism", "benchmark", "--strict", "--profile", "hosted-ci"]).unwrap();
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
fn bundled_font_output_is_truthful_and_legacy_family_automation_remains_compatible() {
    let mut workspace = Workspace::new(Document::new("Bundled font", 320, 200), None);
    workspace
        .execute(Command::AddText {
            text: "Legacy automation".into(),
            name: None,
            font_size: 32.0,
            color: [255; 4],
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    let output = typography::font_list(&workspace.document, None);
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

    let cli = Cli::try_parse_from([
        "prism",
        "--project",
        "type.prism",
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
    let cli = Cli::try_parse_from([
        "prism",
        "--project",
        "type.prism",
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
fn font_source_cli_requires_one_embedded_asset() {
    let cli =
        Cli::try_parse_from(["prism", "--project", "type.prism", "font-source", "12"]).unwrap();
    let CliCommand::FontSource { font_id } = cli.command else {
        panic!("font-source subcommand should parse");
    };
    assert_eq!(font_id, 12);
}

#[test]
fn font_subset_plan_cli_requires_one_embedded_asset() {
    let cli = Cli::try_parse_from(["prism", "--project", "type.prism", "font-subset-plan", "12"])
        .unwrap();
    let CliCommand::FontSubsetPlan { font_id } = cli.command else {
        panic!("font-subset-plan subcommand should parse");
    };
    assert_eq!(font_id, 12);
}

#[test]
fn durable_font_subset_plan_replays_tail_text_without_writes() {
    let directory = temporary_project("font-subset-plan").with_extension("tree");
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("subset-plan.prism");
    let source = directory.join("Hack-Regular.ttf");
    std::fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let mut workspace = Workspace::create_durable(
        Document::new("Subset plan", 320, 200),
        &project,
        cli_actor(),
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: source,
            source_name: None,
        })
        .unwrap();
    let font_id = workspace.document.font_assets[0].id;
    workspace
        .execute(Command::AddText {
            text: "BA\nAV".into(),
            name: None,
            font_size: 48.0,
            color: [255; 4],
            x: 20.0,
            y: 30.0,
        })
        .unwrap();
    let layer_id = workspace.document.selected.unwrap();
    workspace
        .execute(Command::SetTextTypography {
            id: layer_id,
            typography: prism_core::TextTypography {
                font_id: Some(font_id),
                ..Default::default()
            },
        })
        .unwrap();
    drop(workspace);
    let before = tree_snapshot(&directory);

    let output = run(Cli {
        project: project.clone(),
        session: None,
        command: CliCommand::FontSubsetPlan { font_id },
    })
    .unwrap();
    let session_error = run(Cli {
        project,
        session: Some(spectrum_revisions::SessionId::new()),
        command: CliCommand::FontSubsetPlan { font_id },
    })
    .unwrap_err();

    assert_eq!(output["action"], "font_subset_plan");
    assert_eq!(output["mutates_project"], false);
    assert_eq!(output["font_bytes_modified"], false);
    assert_eq!(output["candidate_bytes_emitted"], false);
    assert_eq!(
        output["plan"]["analysis"]["usage"]["layer_ids"],
        json!([layer_id])
    );
    assert_eq!(
        output["plan"]["shaping_samples"][0]["codepoints"],
        json!([66, 65])
    );
    assert_eq!(output["plan"]["physical_replacement_supported"], false);
    assert!(
        session_error
            .to_string()
            .contains("does not accept --session")
    );
    assert_eq!(tree_snapshot(&directory), before);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn font_source_output_proves_identity_without_mutating_the_document() {
    let directory = temporary_project("font-source").with_extension("assets");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    std::fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let mut document = Document::new("Source proof", 320, 200);
    document
        .font_assets
        .push(prism_core::FontAsset::import(1, &source).unwrap());
    let before = document.clone();

    let output = typography::font_source(&document, 1).unwrap();

    assert_eq!(output["action"], "font_source");
    assert_eq!(output["immutable_identity_verified"], true);
    assert_eq!(output["editable_embedding_verified"], true);
    assert_eq!(output["font_bytes_modified"], false);
    assert_eq!(output["mutates_project"], false);
    assert_eq!(
        output["source_bytes"],
        epaint_default_fonts::HACK_REGULAR.len()
    );
    assert_eq!(document, before);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn restricted_font_source_output_is_contextual_and_never_subset_eligible() {
    let directory = temporary_project("restricted-font-source").with_extension("assets");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Restricted.ttf");
    let mut bytes = epaint_default_fonts::HACK_REGULAR.to_vec();
    set_os2_fs_type(&mut bytes, 0x0002);
    std::fs::write(&source, &bytes).unwrap();
    let mut document = Document::new("Restricted source proof", 320, 200);
    document
        .font_assets
        .push(prism_core::FontAsset::import(1, &source).unwrap());

    let output = typography::font_source(&document, 1).unwrap();

    assert_eq!(output["embedding_permission"], "restricted");
    assert_eq!(output["embedding_metadata_allows_subsetting"], false);
    assert_eq!(output["local_editing_supported"], true);
    assert_eq!(output["portable_editable_embedding_verified"], false);
    assert_eq!(output["editable_embedding_verified"], false);
    assert!(
        output["portability_note"]
            .as_str()
            .unwrap()
            .contains("Restricted embedding")
    );
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    std::fs::remove_dir_all(directory).unwrap();
}

fn set_os2_fs_type(bytes: &mut [u8], fs_type: u16) {
    let table_count = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    for index in 0..table_count {
        let record = 12 + index * 16;
        if &bytes[record..record + 4] == b"OS/2" {
            let offset = u32::from_be_bytes([
                bytes[record + 8],
                bytes[record + 9],
                bytes[record + 10],
                bytes[record + 11],
            ]) as usize;
            bytes[offset + 8..offset + 10].copy_from_slice(&fs_type.to_be_bytes());
            return;
        }
    }
    panic!("test font has no OS/2 table");
}

#[test]
fn durable_font_source_keeps_store_bytes_unchanged_and_rejects_sessions() {
    let project = temporary_project("font-source-read-only");
    let directory = temporary_project("font-source-read-only").with_extension("assets");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Hack-Regular.ttf");
    std::fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    run(Cli {
        project: project.clone(),
        session: None,
        command: CliCommand::Init {
            name: "Read-only proof".into(),
            width: 320,
            height: 200,
            background: "18191dff".into(),
        },
    })
    .unwrap();
    run(Cli {
        project: project.clone(),
        session: None,
        command: CliCommand::FontImport { path: source },
    })
    .unwrap();
    let before = std::fs::read(&project).unwrap();

    let output = run(Cli {
        project: project.clone(),
        session: None,
        command: CliCommand::FontSource { font_id: 1 },
    })
    .unwrap();
    let session_error = run(Cli {
        project: project.clone(),
        session: Some(spectrum_revisions::SessionId::new()),
        command: CliCommand::FontSource { font_id: 1 },
    })
    .unwrap_err();

    assert_eq!(output["mutates_project"], false);
    assert!(
        session_error
            .to_string()
            .contains("does not accept --session")
    );
    assert_eq!(std::fs::read(&project).unwrap(), before);
    std::fs::remove_file(project).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_font_source_ignores_newer_live_and_recovery_state_without_writes() {
    let directory = temporary_project("font-source-invariant").with_extension("tree");
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("invariant.prism");
    let source = directory.join("Hack-Regular.ttf");
    std::fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    run(Cli {
        project: project.clone(),
        session: None,
        command: CliCommand::Init {
            name: "Immutable inspection".into(),
            width: 320,
            height: 200,
            background: "18191dff".into(),
        },
    })
    .unwrap();
    run(Cli {
        project: project.clone(),
        session: None,
        command: CliCommand::FontImport { path: source },
    })
    .unwrap();
    let project_info = spectrum_revisions::RevisionStore::open_read_only(&project)
        .unwrap()
        .project_info()
        .unwrap();
    let live_cache = directory.join(".revision-cache");
    drop(
        spectrum_revisions::LiveRevisionStore::open(&project, &live_cache)
            .expect("test live cache should materialize from the canonical project"),
    );
    let working = live_cache
        .join(project_info.project_id.to_string())
        .join("live.sqlite");
    let canonical_writer = rusqlite::Connection::open(&project).unwrap();
    canonical_writer
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE read_only_recovery_probe(value INTEGER);
             INSERT INTO read_only_recovery_probe VALUES (1);
             UPDATE spectrum_meta SET value = CAST('888888' AS BLOB)
             WHERE key = 'storage_generation';",
        )
        .unwrap();
    let live_writer = rusqlite::Connection::open(working).unwrap();
    live_writer
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE newer_live_probe(value INTEGER);
             INSERT INTO newer_live_probe VALUES (2);
             UPDATE spectrum_meta SET value = CAST('999999' AS BLOB)
             WHERE key = 'storage_generation';",
        )
        .unwrap();
    let before = tree_snapshot(&directory);

    let output = run(Cli {
        project: project.clone(),
        session: None,
        command: CliCommand::FontSource { font_id: 1 },
    })
    .unwrap();

    assert_eq!(output["immutable_identity_verified"], true);
    assert_eq!(tree_snapshot(&directory), before);
    drop(live_writer);
    drop(canonical_writer);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn durable_font_source_replays_transferred_fonts_with_dedup_without_writes() {
    let directory = temporary_project("font-source-transfer").with_extension("tree");
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("transfer-destination.prism");
    let source = directory.join("Hack-Regular.ttf");
    let transfer_path = directory.join("font-layer.json");
    std::fs::write(&source, epaint_default_fonts::HACK_REGULAR).unwrap();
    let mut source_workspace = Workspace::new(Document::new("Transfer source", 320, 200), None);
    source_workspace
        .execute(Command::ImportFont {
            path: source.clone(),
            source_name: None,
        })
        .unwrap();
    let font_id = source_workspace.document.font_assets[0].id;
    source_workspace
        .execute(Command::AddText {
            text: "Transferred type".into(),
            name: None,
            font_size: 48.0,
            color: [255, 255, 255, 255],
            x: 20.0,
            y: 30.0,
        })
        .unwrap();
    let layer_id = source_workspace.document.selected.unwrap();
    source_workspace
        .execute(Command::SetTextTypography {
            id: layer_id,
            typography: prism_core::TextTypography {
                font_id: Some(font_id),
                ..Default::default()
            },
        })
        .unwrap();
    let transfer = prism_core::LayerTransfer::from_selected(&source_workspace.document).unwrap();
    std::fs::write(&transfer_path, transfer.to_json().unwrap()).unwrap();
    run(Cli {
        project: project.clone(),
        session: None,
        command: CliCommand::Init {
            name: "Transfer destination".into(),
            width: 320,
            height: 200,
            background: "18191dff".into(),
        },
    })
    .unwrap();
    for _ in 0..2 {
        run(Cli {
            project: project.clone(),
            session: None,
            command: CliCommand::LayerPaste(LayerPasteArgs {
                input: transfer_path.clone(),
                index: None,
            }),
        })
        .unwrap();
    }
    let before = tree_snapshot(&directory);

    let output = run(Cli {
        project: project.clone(),
        session: None,
        command: CliCommand::FontSource { font_id: 1 },
    })
    .unwrap();
    let inspected = prism_core::inspect_font_source_read_only(&project, 1).unwrap();

    assert_eq!(output["family"], "Hack");
    assert_eq!(output["style"], "Regular");
    assert_eq!(output["source_name"], "Hack-Regular.ttf");
    assert_eq!(
        output["content_hash"],
        spectrum_revisions::AssetId::for_bytes(epaint_default_fonts::HACK_REGULAR).to_string()
    );
    assert_eq!(inspected.embedded_font_count, 1);
    assert_eq!(inspected.next_font_id, 2);
    assert_eq!(inspected.font.id, 1);
    assert_eq!(tree_snapshot(&directory), before);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn font_usage_output_limits_its_non_mutation_and_coverage_claims() {
    let output = typography::font_usage(&Document::new("Usage", 320, 200), None).unwrap();
    assert_eq!(output["action"], "font_usage");
    assert_eq!(output["analysis_scope"], "unicode_cmap_subset_retention");
    assert_eq!(output["font_bytes_modified"], false);
    assert!(output.get("mutates_project").is_none());
    assert_eq!(output["editable_font_bytes_preserved"], true);
    assert_eq!(output["limitations"].as_array().unwrap().len(), 3);
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

pub(super) fn temporary_project(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!("prism-{label}-cli-{stamp}.prism"))
}

fn tree_snapshot(root: &Path) -> Vec<(PathBuf, bool, Vec<u8>)> {
    fn visit(root: &Path, directory: &Path, snapshot: &mut Vec<(PathBuf, bool, Vec<u8>)>) {
        let mut entries = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            let relative = path.strip_prefix(root).unwrap().to_owned();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                snapshot.push((relative, true, Vec::new()));
                visit(root, &path, snapshot);
            } else {
                snapshot.push((relative, false, std::fs::read(path).unwrap()));
            }
        }
    }

    let mut snapshot = Vec::new();
    visit(root, root, &mut snapshot);
    snapshot
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
