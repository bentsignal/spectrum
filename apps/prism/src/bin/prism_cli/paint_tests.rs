use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_path(label: &str, extension: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-paint-{label}-{stamp}.{extension}"))
}

fn invoke(project: &Path, arguments: &[&str]) -> Result<Value> {
    let mut cli = vec!["prism", "--project", project.to_str().unwrap()];
    cli.extend_from_slice(arguments);
    run(Cli::try_parse_from(cli).unwrap())
}

fn stroke_json(mode: &str, x: f32, y: f32) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "style": {
            "mode": mode,
            "color": [20, 180, 240, 255],
            "size": 12.0,
            "hardness": 0.8,
            "opacity": 1.0,
            "spacing": 0.15
        },
        "samples": [{"x": x, "y": y, "pressure": 1.0}]
    }))
    .unwrap()
}

#[test]
fn paint_cli_persists_each_stroke_and_honors_no_selection() {
    let project = temporary_path("e2e", "prism");
    let selected_stroke = temporary_path("selected", "json");
    let unselected_stroke = temporary_path("unselected", "json");
    std::fs::write(&selected_stroke, stroke_json("paint", 12.5, 12.5)).unwrap();
    std::fs::write(&unselected_stroke, stroke_json("erase", 20.5, 20.5)).unwrap();

    invoke(
        &project,
        &["init", "Paint CLI", "--width", "64", "--height", "64"],
    )
    .unwrap();
    invoke(&project, &["selection", "rectangle", "8", "8", "16", "16"]).unwrap();
    invoke(
        &project,
        &[
            "paint",
            "add-layer",
            "--name",
            "Ink",
            "--width",
            "64",
            "--height",
            "64",
        ],
    )
    .unwrap();
    invoke(
        &project,
        &["paint", "stroke", "1", selected_stroke.to_str().unwrap()],
    )
    .unwrap();
    invoke(
        &project,
        &[
            "paint",
            "stroke",
            "1",
            unselected_stroke.to_str().unwrap(),
            "--no-selection",
        ],
    )
    .unwrap();

    let document = Workspace::load_read_only(&project).unwrap();
    let prism_core::LayerKind::Paint { program } = &document.layer(1).unwrap().kind else {
        panic!("paint CLI did not create a Paint layer")
    };
    assert_eq!(program.strokes.len(), 2);
    assert!(program.strokes[0].clip.is_some());
    assert!(program.strokes[1].clip.is_none());

    invoke(&project, &["run", r#"{"command":"undo"}"#]).unwrap();
    let document = Workspace::load_read_only(&project).unwrap();
    let prism_core::LayerKind::Paint { program } = &document.layer(1).unwrap().kind else {
        panic!("undo removed the Paint layer instead of one stroke")
    };
    assert_eq!(program.strokes.len(), 1);

    for path in [project, selected_stroke, unselected_stroke] {
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn paint_cli_rejects_invalid_and_oversized_stroke_files_without_mutation() {
    let project = temporary_path("invalid", "prism");
    let invalid = temporary_path("invalid", "json");
    let oversized = temporary_path("oversized", "json");
    invoke(
        &project,
        &["init", "Paint CLI", "--width", "32", "--height", "32"],
    )
    .unwrap();
    invoke(
        &project,
        &["paint", "add-layer", "--width", "32", "--height", "32"],
    )
    .unwrap();

    std::fs::write(&invalid, br#"{"style":{},"samples":[]}"#).unwrap();
    let error = invoke(
        &project,
        &["paint", "stroke", "1", invalid.to_str().unwrap()],
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("invalid BrushStroke JSON"));

    let file = std::fs::File::create(&oversized).unwrap();
    file.set_len((paint::MAX_BRUSH_STROKE_JSON_BYTES as u64) + 1)
        .unwrap();
    let error = invoke(
        &project,
        &["paint", "stroke", "1", oversized.to_str().unwrap()],
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("32 MiB input limit"));

    let document = Workspace::load_read_only(&project).unwrap();
    let prism_core::LayerKind::Paint { program } = &document.layer(1).unwrap().kind else {
        panic!("expected Paint layer")
    };
    assert!(program.strokes.is_empty());
    for path in [project, invalid, oversized] {
        std::fs::remove_file(path).unwrap();
    }
}
