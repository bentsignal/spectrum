use super::*;

fn temporary_project(label: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-cli-gradient-{label}-{stamp}.prism"))
}

fn invoke(project: &std::path::Path, arguments: &[&str]) -> anyhow::Result<()> {
    let mut argv = vec!["prism", "--project", project.to_str().unwrap()];
    argv.extend_from_slice(arguments);
    run(Cli::try_parse_from(argv).unwrap()).map(|_| ())
}

#[test]
fn legacy_and_modern_gradient_cli_share_the_set_shape_fill_command() {
    let project = temporary_project("surface");
    invoke(
        &project,
        &["init", "Gradient CLI", "--width", "80", "--height", "60"],
    )
    .unwrap();
    invoke(
        &project,
        &["add-rectangle", "--width", "40", "--height", "30"],
    )
    .unwrap();
    invoke(
        &project,
        &[
            "gradient", "1", "--angle", "23", "--start", "ff0000ff", "--end", "0000ff80",
        ],
    )
    .unwrap();
    let legacy = Workspace::load_read_only(&project).unwrap();
    let Some(prism_core::ShapeFill::Gradient(gradient)) = &legacy.layer(1).unwrap().shape_fill
    else {
        panic!("legacy CLI did not set a gradient")
    };
    assert_eq!(gradient.kind, prism_core::GradientKind::Linear);
    assert_eq!(gradient.stops.len(), 2);
    assert_eq!(
        prism_core::required_command_operations_version(&[Command::SetShapeFill {
            id: 1,
            fill: legacy.layer(1).unwrap().shape_fill.clone(),
        }]),
        3
    );

    invoke(
        &project,
        &[
            "gradient",
            "1",
            "--kind",
            "radial",
            "--spread",
            "reflect",
            "--center-x",
            "0.4",
            "--center-y",
            "0.6",
            "--radius",
            "0.75",
            "--stop",
            "0:ff0000ff",
            "--stop",
            "0.35:00ff00c0",
            "--stop",
            "1:0000ff00",
        ],
    )
    .unwrap();
    let modern = Workspace::load_read_only(&project).unwrap();
    let Some(prism_core::ShapeFill::Gradient(gradient)) = &modern.layer(1).unwrap().shape_fill
    else {
        panic!("modern CLI did not set a gradient")
    };
    assert_eq!(gradient.kind, prism_core::GradientKind::Radial);
    assert_eq!(gradient.spread, prism_core::GradientSpread::Reflect);
    assert_eq!(gradient.center, [0.4, 0.6]);
    assert_eq!(gradient.radius, 0.75);
    assert_eq!(gradient.stops.len(), 3);
    std::fs::remove_file(project).unwrap();
}

#[test]
fn invalid_cli_gradient_is_atomic() {
    let project = temporary_project("invalid");
    invoke(
        &project,
        &["init", "Atomic", "--width", "40", "--height", "40"],
    )
    .unwrap();
    invoke(
        &project,
        &["add-ellipse", "--width", "20", "--height", "20"],
    )
    .unwrap();
    let before = Workspace::load_read_only(&project).unwrap();
    let result = invoke(
        &project,
        &[
            "gradient",
            "1",
            "--stop",
            "0.5:ff0000ff",
            "--stop",
            "0.5:0000ffff",
        ],
    );
    assert!(result.is_err());
    assert_eq!(Workspace::load_read_only(&project).unwrap(), before);
    std::fs::remove_file(project).unwrap();
}
