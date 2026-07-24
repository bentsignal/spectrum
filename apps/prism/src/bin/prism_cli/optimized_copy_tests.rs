use super::*;

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
