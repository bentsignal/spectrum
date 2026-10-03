use super::*;

fn invoke(project: &std::path::Path, arguments: &[&str]) -> anyhow::Result<()> {
    let mut argv = vec!["prism", "--document", project.to_str().unwrap()];
    argv.extend_from_slice(arguments);
    run(Cli::try_parse_from(argv).unwrap()).map(|_| ())
}

#[test]
fn effect_cli_sets_one_style_and_keeps_the_others() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let project = std::env::temp_dir().join(format!("prism-cli-effects-{stamp}.prism"));
    invoke(
        &project,
        &["init", "Effects", "--width", "80", "--height", "60"],
    )
    .unwrap();
    invoke(
        &project,
        &["add-rectangle", "--width", "40", "--height", "30"],
    )
    .unwrap();
    invoke(&project, &["shadow", "1", "--blur", "4"]).unwrap();
    invoke(
        &project,
        &[
            "effect",
            "1",
            "stroke",
            "--size",
            "5",
            "--position",
            "center",
            "--color",
            "ff0000ff",
        ],
    )
    .unwrap();
    invoke(&project, &["effect", "1", "outer-glow", "--spread", "0.5"]).unwrap();
    invoke(&project, &["effect", "1", "color-overlay"]).unwrap();
    invoke(&project, &["effect", "1", "inner-shadow", "--x", "-3"]).unwrap();
    invoke(&project, &["effect", "1", "stroke", "--size", "2"]).unwrap();
    let style = Workspace::load_read_only(&project)
        .unwrap()
        .layer(1)
        .unwrap()
        .style
        .clone();
    let stroke = style.stroke.unwrap();
    assert_eq!(stroke.size, 2.0, "later edits change only what they name");
    assert_eq!(stroke.position, prism_core::StrokePosition::Center);
    assert_eq!(stroke.color, [255, 0, 0, 255]);
    assert_eq!(style.outer_glow.unwrap().spread, 0.5);
    assert_eq!(style.inner_shadow.unwrap().offset_x, -3.0);
    assert!(style.color_overlay.is_some());
    assert_eq!(style.drop_shadow.unwrap().blur_radius, 4.0, "shadow stays");

    invoke(
        &project,
        &[
            "effect",
            "1",
            "bevel",
            "--bevel-style",
            "emboss",
            "--angle",
            "45",
            "--down",
            "true",
        ],
    )
    .unwrap();
    invoke(
        &project,
        &[
            "effect",
            "1",
            "satin",
            "--mode",
            "screen",
            "--distance",
            "6",
        ],
    )
    .unwrap();
    invoke(
        &project,
        &[
            "effect",
            "1",
            "gradient-overlay",
            "--stop",
            "0:ff0000ff",
            "--stop",
            "0.5:00ff00ff",
            "--stop",
            "1:0000ffff",
            "--gradient-kind",
            "radial",
        ],
    )
    .unwrap();
    let style = Workspace::load_read_only(&project)
        .unwrap()
        .layer(1)
        .unwrap()
        .style
        .clone();
    let bevel = style.bevel.unwrap();
    assert_eq!(bevel.style, prism_core::BevelStyle::Emboss);
    assert!(!bevel.up && bevel.angle == 45.0);
    assert_eq!(
        style.satin.unwrap().blend_mode,
        prism_core::BlendMode::Screen
    );
    let overlay = style.gradient_overlay.unwrap();
    assert_eq!(overlay.gradient.stops.len(), 3);
    assert_eq!(overlay.gradient.kind, prism_core::GradientKind::Radial);
    assert!(
        invoke(
            &project,
            &["effect", "1", "gradient-overlay", "--stop", "0:ffffffff"]
        )
        .is_err()
    );

    invoke(&project, &["effect", "1", "outer-glow", "--clear"]).unwrap();
    invoke(&project, &["shadow", "1", "--clear"]).unwrap();
    let style = Workspace::load_read_only(&project)
        .unwrap()
        .layer(1)
        .unwrap()
        .style
        .clone();
    assert!(style.outer_glow.is_none() && style.drop_shadow.is_none());
    assert!(style.stroke.is_some(), "clearing one style keeps the rest");
    std::fs::remove_file(project).unwrap();
}

#[test]
fn selection_cli_selects_ellipses_inverts_and_masks_layers() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let project = std::env::temp_dir().join(format!("prism-cli-selection-{stamp}.prism"));
    invoke(
        &project,
        &["init", "Selections", "--width", "80", "--height", "60"],
    )
    .unwrap();
    invoke(
        &project,
        &["add-rectangle", "--width", "40", "--height", "30"],
    )
    .unwrap();
    invoke(&project, &["selection", "ellipse", "10", "10", "30", "20"]).unwrap();
    let selected = Workspace::load_read_only(&project).unwrap();
    assert!(selected.selection.is_some());
    invoke(&project, &["selection", "invert"]).unwrap();
    let inverted = Workspace::load_read_only(&project).unwrap();
    let (.., width, height) = inverted.selection.as_ref().unwrap().bounds();
    assert_eq!((width, height), (80, 60), "the inverse spans the canvas");
    invoke(&project, &["selection", "mask", "1"]).unwrap();
    let masked = Workspace::load_read_only(&project).unwrap();
    assert!(masked.layer(1).unwrap().vector_mask.is_some());
    std::fs::remove_file(project).unwrap();
}

#[test]
fn sample_cli_reads_the_composited_color() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let project = std::env::temp_dir().join(format!("prism-cli-sample-{stamp}.prism"));
    invoke(
        &project,
        &["init", "Sample", "--width", "40", "--height", "30"],
    )
    .unwrap();
    invoke(
        &project,
        &[
            "add-rectangle",
            "--width",
            "10",
            "--height",
            "10",
            "--color",
            "ff0000ff",
        ],
    )
    .unwrap();
    let document = Workspace::load_read_only(&project).unwrap();
    assert_eq!(
        prism_core::sample_document_color(&document, 5, 5).unwrap(),
        [255, 0, 0, 255]
    );
    assert!(prism_core::sample_document_color(&document, 50, 5).is_err());
    let sampled = run(Cli::try_parse_from([
        "prism",
        "--document",
        project.to_str().unwrap(),
        "sample",
        "5",
        "5",
    ])
    .unwrap())
    .unwrap();
    assert_eq!(sampled["color"], "ff0000ff");
    std::fs::remove_file(project).unwrap();
}
