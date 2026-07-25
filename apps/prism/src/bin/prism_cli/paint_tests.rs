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
fn clone_cli_captures_one_raster_source_and_commits_resolved_stroke() {
    let directory = std::fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!(
            "prism-clone-cli-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("clone.prism");
    let source = directory.join("source.png");
    let stroke = directory.join("stroke.json");
    let export = directory.join("clone-export.png");
    let mut image = image::RgbaImage::new(8, 8);
    image.put_pixel(1, 1, image::Rgba([210, 40, 90, 255]));
    image.save(&source).unwrap();
    std::fs::write(&stroke, stroke_json("paint", 4.5, 4.5)).unwrap();

    invoke(
        &project,
        &["init", "Clone CLI", "--width", "8", "--height", "8"],
    )
    .unwrap();
    invoke(&project, &["add-image", source.to_str().unwrap()]).unwrap();
    invoke(
        &project,
        &["paint", "add-layer", "--width", "8", "--height", "8"],
    )
    .unwrap();
    invoke(&project, &["paint", "clone-source", "1", "1.5", "1.5"]).unwrap();
    invoke(
        &project,
        &[
            "paint",
            "clone-stroke",
            "2",
            stroke.to_str().unwrap(),
            "--no-selection",
        ],
    )
    .unwrap();

    let document = Workspace::load_read_only(&project).unwrap();
    let prism_core::LayerKind::Paint { program } = &document.layer(2).unwrap().kind else {
        panic!("clone CLI did not retain a Paint layer")
    };
    assert_eq!(program.version, prism_core::BRUSH_PROGRAM_VERSION);
    assert_eq!(program.strokes.len(), 1);
    assert_eq!(
        program.strokes[0].style.mode,
        prism_core::BrushMode::CloneStamp
    );
    assert!(program.strokes[0].sampled_source_identity().is_some());
    let mut rendered_document = document.clone();
    rendered_document.layer_mut(1).unwrap().visible = false;
    let rendered = prism_core::render_document(&rendered_document, None)
        .unwrap()
        .to_rgba8();
    assert_eq!(rendered.get_pixel(4, 4).0, [210, 40, 90, 255]);
    invoke(&project, &["visibility", "1", "false"]).unwrap();
    invoke(
        &project,
        &["export", export.to_str().unwrap(), "--quality", "92"],
    )
    .unwrap();
    assert_eq!(image::open(export).unwrap().to_rgba8(), rendered);
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
