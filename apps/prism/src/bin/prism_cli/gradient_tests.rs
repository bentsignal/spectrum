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

#[test]
fn modern_stops_and_legacy_endpoints_are_mutually_exclusive() {
    let project = temporary_project("surface-conflict");
    invoke(
        &project,
        &["init", "Conflict", "--width", "32", "--height", "32"],
    )
    .unwrap();
    invoke(
        &project,
        &["add-rectangle", "--width", "16", "--height", "16"],
    )
    .unwrap();
    let before = Workspace::load_read_only(&project).unwrap();
    for legacy in [["--start", "00ff00ff"], ["--end", "ffffffff"]] {
        let arguments = [
            "prism",
            "--project",
            project.to_str().unwrap(),
            "gradient",
            "1",
            "--stop",
            "0:ff0000ff",
            "--stop",
            "1:0000ffff",
            legacy[0],
            legacy[1],
        ];
        assert!(Cli::try_parse_from(arguments).is_err());
        assert_eq!(Workspace::load_read_only(&project).unwrap(), before);
    }
    std::fs::remove_file(project).unwrap();
}

#[test]
fn structured_gradient_json_is_bounded_strict_and_exclusive() {
    let project = temporary_project("structured");
    invoke(
        &project,
        &["init", "Structured", "--width", "200", "--height", "100"],
    )
    .unwrap();
    invoke(
        &project,
        &["add-rectangle", "--width", "200", "--height", "100"],
    )
    .unwrap();
    let before = Workspace::load_read_only(&project).unwrap();
    let valid = r#"{"kind":"angle","angle":45,"stops":[{"position":0,"color":[255,0,0,255]},{"position":0.5,"color":[0,255,0,128]},{"position":1,"color":[0,0,255,0]}],"center":[0.4,0.6],"spread":"reflect","interpolation":"premultiplied_srgb_v1","offset":0.125,"extent":0.75}"#;
    invoke(&project, &["gradient", "1", "--gradient-json", valid]).unwrap();
    let current = Workspace::load_read_only(&project).unwrap();
    let Some(prism_core::ShapeFill::Gradient(gradient)) = &current.layer(1).unwrap().shape_fill
    else {
        panic!("structured JSON did not set a gradient")
    };
    assert_eq!(gradient.kind, prism_core::GradientKind::Angle);
    assert_eq!(
        gradient.interpolation,
        prism_core::GradientInterpolation::PremultipliedSrgbV1
    );
    assert_eq!(gradient.offset, 0.125);
    assert_eq!(gradient.extent, 0.75);
    assert_eq!(gradient.stops[2].color, [0; 4]);

    let after_valid = current.clone();
    for invalid in [
        r#"{"kind":"radial","raduis":0.9,"stops":[{"position":0,"color":[0,0,0,255]},{"position":1,"color":[255,255,255,255]}],"interpolation":"premultiplied_srgb_v1"}"#,
        r#"{"kind":"radial","kind":"angle","stops":[{"position":0,"color":[0,0,0,255]},{"position":1,"color":[255,255,255,255]}],"interpolation":"premultiplied_srgb_v1"}"#,
        r#"{"kind":"radial","stops":[{"position":0,"opacity":1,"color":[0,0,0,255]},{"position":1,"color":[255,255,255,255]}],"interpolation":"premultiplied_srgb_v1"}"#,
        r#"{"kind":"radial","stops":[{"position":0,"color":[0,0,0,255]},{"position":1,"color":[255,255,255,255]}],"interpolation":"linear_rgb_v1"}"#,
    ] {
        assert!(
            invoke(&project, &["gradient", "1", "--gradient-json", invalid]).is_err(),
            "invalid structured gradient was accepted: {invalid}"
        );
        assert_eq!(Workspace::load_read_only(&project).unwrap(), after_valid);
    }

    let oversized = format!(
        r#"{{"kind":"radial","interpolation":"premultiplied_srgb_v1","unknown":"{}","stops":[]}}"#,
        "x".repeat(17 * 1024)
    );
    assert!(invoke(&project, &["gradient", "1", "--gradient-json", &oversized]).is_err());
    assert_eq!(Workspace::load_read_only(&project).unwrap(), after_valid);

    for arguments in [
        vec!["gradient", "1", "--clear", "--kind", "radial"],
        vec!["gradient", "1", "--clear", "--start", "ff0000ff"],
        vec!["gradient", "1", "--gradient-json", valid, "--radius", "0.7"],
    ] {
        let mut argv = vec!["prism", "--project", project.to_str().unwrap()];
        argv.extend(arguments);
        assert!(Cli::try_parse_from(argv).is_err());
        assert_eq!(Workspace::load_read_only(&project).unwrap(), after_valid);
    }
    assert_ne!(after_valid, before);
    std::fs::remove_file(project).unwrap();
}

#[test]
fn required_live_structured_gradient_never_falls_back_to_direct_mutation() {
    let project = temporary_project("structured-required-live");
    let human_session = spectrum_revisions::SessionId::new();
    let mut human = Workspace::create_durable(
        prism_core::Document::new("Required structured", 200, 100),
        &project,
        spectrum_revisions::Actor {
            id: "human:structured-gradient".into(),
            display_name: "Structured gradient".into(),
            kind: spectrum_revisions::ActorKind::Human,
        },
        human_session,
    )
    .unwrap();
    human
        .execute(Command::AddRectangle {
            name: None,
            width: 200,
            height: 100,
            color: [255; 4],
            corner_radius: 0.0,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    human.save(None).unwrap();
    drop(human);
    let before = Workspace::load_read_only(&project).unwrap();
    let collaboration = Workspace::start_collaboration(
        &project,
        Some(human_session),
        spectrum_revisions::Actor {
            id: "external-agent:structured-gradient".into(),
            display_name: "Structured gradient agent".into(),
            kind: spectrum_revisions::ActorKind::Agent,
        },
        spectrum_revisions::CollaborationMode::Separate,
    )
    .unwrap();
    let json = r#"{"kind":"radial","stops":[{"position":0,"color":[255,0,0,255]},{"position":1,"color":[0,0,0,0]}],"interpolation":"premultiplied_srgb_v1"}"#;
    let error = run(Cli::try_parse_from([
        "prism",
        "--project",
        project.to_str().unwrap(),
        "--session",
        &collaboration.agent_session.to_string(),
        "--live",
        "required",
        "gradient",
        "1",
        "--gradient-json",
        json,
    ])
    .unwrap())
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no authenticated live Prism binding")
    );
    assert_eq!(Workspace::load_read_only(&project).unwrap(), before);
    std::fs::remove_file(project).unwrap();
}

#[test]
fn radial_and_angle_numeric_boundaries_fail_closed_or_export_without_panicking() {
    let project = temporary_project("numeric-boundaries");
    invoke(
        &project,
        &["init", "Boundaries", "--width", "64", "--height", "64"],
    )
    .unwrap();
    invoke(
        &project,
        &["add-rectangle", "--width", "64", "--height", "64"],
    )
    .unwrap();
    let before = Workspace::load_read_only(&project).unwrap();

    let invalid: &[&[&str]] = &[
        &[
            "gradient",
            "1",
            "--kind",
            "radial",
            "--spread",
            "repeat",
            "--radius",
            "1e-45",
            "--stop",
            "0:ff0000ff",
            "--stop",
            "1:0000ffff",
        ],
        &[
            "gradient",
            "1",
            "--kind",
            "radial",
            "--spread",
            "reflect",
            "--radius",
            "5e-39",
            "--stop",
            "0:ff0000ff",
            "--stop",
            "1:0000ffff",
        ],
        &["gradient", "1", "--angle", "NaN"],
        &["gradient", "1", "--angle", "inf"],
        &["gradient", "1", "--center-x", "3.4028235e38"],
        &["gradient", "1", "--radius", "inf"],
    ];
    for arguments in invalid {
        assert!(
            invoke(&project, arguments).is_err(),
            "invalid gradient invocation was accepted: {arguments:?}"
        );
        assert_eq!(Workspace::load_read_only(&project).unwrap(), before);
    }

    let safe_cases: &[&[&str]] = &[
        &[
            "gradient",
            "1",
            "--kind",
            "radial",
            "--spread",
            "reflect",
            "--radius",
            "1.17549435e-38",
            "--stop",
            "0:ff0000ff",
            "--stop",
            "1:0000ffff",
        ],
        &[
            "gradient",
            "1",
            "--kind",
            "radial",
            "--spread",
            "repeat",
            "--radius",
            "3.4028235e38",
            "--stop",
            "0:ff0000ff",
            "--stop",
            "1:0000ffff",
        ],
        &[
            "gradient",
            "1",
            "--kind",
            "linear",
            "--spread",
            "reflect",
            "--angle",
            "3.4028235e38",
            "--stop",
            "0:ff0000ff",
            "--stop",
            "1:0000ffff",
        ],
    ];
    for (index, arguments) in safe_cases.iter().enumerate() {
        invoke(&project, arguments).unwrap();
        let export = project.with_extension(format!("boundary-{index}.png"));
        invoke(&project, &["export", export.to_str().unwrap()]).unwrap();
        assert!(std::fs::metadata(&export).unwrap().len() > 0);
        std::fs::remove_file(export).unwrap();
    }

    std::fs::remove_file(project).unwrap();
}
