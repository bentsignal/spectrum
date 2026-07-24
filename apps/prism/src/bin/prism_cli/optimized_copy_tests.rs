use super::*;

const STATIC_FONT: &[u8] =
    include_bytes!("../../../../../crates/spectrum-fonts/tests/fonts/noto-sans-static-source.ttf");

#[test]
fn optimized_copy_cli_requires_an_output_path() {
    let cli = Cli::try_parse_from([
        "prism",
        "--project",
        "source.prism",
        "optimized-copy",
        "--output",
        "smaller.prism",
    ])
    .unwrap();

    let CliCommand::OptimizedCopy { output } = cli.command else {
        panic!("optimized-copy subcommand should parse");
    };
    assert_eq!(output, PathBuf::from("smaller.prism"));
}

#[test]
fn optimized_copy_rejects_session_before_reading_the_source() {
    let error = run(Cli {
        project: PathBuf::from("missing.prism"),
        session: Some(SessionId::new()),
        command: CliCommand::OptimizedCopy {
            output: PathBuf::from("unused.prism"),
        },
    })
    .unwrap_err();

    assert!(error.to_string().contains("does not accept --session"));
}

#[test]
fn optimized_copy_cli_dispatches_end_to_end_without_mutating_the_source() {
    let directory = std::fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!("prism-cli-optimized-copy-{}", SessionId::new()));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("source.prism");
    let output = directory.join("optimized.prism");
    let font = directory.join("NotoSans.ttf");
    std::fs::write(&font, STATIC_FONT).unwrap();
    let mut workspace = Workspace::create_durable(
        Document::new("CLI optimized copy", 160, 90),
        &source,
        cli_actor(),
        SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: font,
            source_name: None,
        })
        .unwrap();
    let font_id = workspace.document.font_assets[0].id;
    workspace
        .execute(Command::AddText {
            text: "AV".into(),
            name: None,
            font_size: 28.0,
            color: [255; 4],
            x: 4.0,
            y: 4.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetTextTypography {
            id: workspace.document.selected.unwrap(),
            typography: prism_core::TextTypography {
                font_id: Some(font_id),
                ..Default::default()
            },
        })
        .unwrap();
    drop(workspace);
    let source_before = std::fs::read(&source).unwrap();

    let result = run(Cli {
        project: source.clone(),
        session: None,
        command: CliCommand::OptimizedCopy {
            output: output.clone(),
        },
    })
    .unwrap();

    assert_eq!(result["action"], "optimized_copy");
    assert_eq!(
        result["report"]["output"],
        output.to_string_lossy().as_ref()
    );
    assert_eq!(std::fs::read(&source).unwrap(), source_before);
    assert!(output.metadata().unwrap().len() < source.metadata().unwrap().len());
    let destination = spectrum_revisions::RevisionStore::open_read_only(&output).unwrap();
    destination.verify_integrity().unwrap();
    assert_eq!(
        destination.project_info().unwrap().application_id,
        "spectrum.prism"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
