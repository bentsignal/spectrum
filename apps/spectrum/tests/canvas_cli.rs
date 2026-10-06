//! Canvas editing through the CLI. Tests name a canvas by a path; the
//! helpers keep each one as an asset in a library beside it.
use std::{path::Path, process::Command as ProcessCommand};

use serde_json::Value;
use spectrum_canvas::Document;

#[test]
fn cli_creates_and_styles_editable_ellipses() {
    let directory = std::env::temp_dir().join(format!("canvas-cli-ellipse-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("ellipse.spectrum");
    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "init",
        "Ellipse CLI",
        "--width",
        "640",
        "--height",
        "480",
    ]);
    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "add-ellipse",
        "--name",
        "Sun",
        "--width",
        "180",
        "--height",
        "120",
        "--color",
        "f7b266ff",
        "--x",
        "40",
        "--y",
        "50",
    ]);
    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "stroke",
        "1",
        "--width",
        "8",
        "--color",
        "ffffffff",
    ]);
    let listed = run_canvas(&["--document", project.to_str().unwrap(), "inspect"]);
    let layer = &listed["document"]["layers"][0];
    assert_eq!(layer["kind"]["type"], "ellipse");
    assert_eq!(layer["stroke"]["enabled"], true);
    assert_eq!(layer["stroke"]["width"], 8.0);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cli_magic_wand_supports_contiguous_and_canvas_wide_matching() {
    let directory = std::env::temp_dir().join(format!("canvas-cli-wand-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("wand.spectrum");
    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "init",
        "Wand CLI",
        "--width",
        "20",
        "--height",
        "8",
    ]);
    for x in [2, 14] {
        run_canvas(&[
            "--document",
            project.to_str().unwrap(),
            "add-rectangle",
            "--width",
            "3",
            "--height",
            "3",
            "--radius",
            "0",
            "--color",
            "e02030ff",
            "--x",
            &x.to_string(),
            "--y",
            "2",
        ]);
    }
    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "selection",
        "magic-wand",
        "2",
        "2",
        "--tolerance",
        "0",
        "--no-antialias",
    ]);
    let contiguous = run_canvas(&["--document", project.to_str().unwrap(), "inspect"]);
    assert_eq!(contiguous["document"]["selection"]["type"], "rectangle");
    assert_eq!(contiguous["document"]["selection"]["width"], 3);

    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "selection",
        "magic-wand",
        "2",
        "2",
        "--tolerance",
        "0",
        "--noncontiguous",
        "--no-antialias",
    ]);
    let global = run_canvas(&["--document", project.to_str().unwrap(), "inspect"]);
    assert_eq!(global["document"]["selection"]["type"], "color_mask");
    assert_eq!(global["document"]["selection"]["x"], 2);
    assert_eq!(global["document"]["selection"]["width"], 15);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cli_magic_wand_delete_is_nondestructive_and_durable() {
    let directory =
        std::env::temp_dir().join(format!("canvas-cli-wand-delete-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("two-colors.png");
    image::RgbaImage::from_fn(8, 4, |x, _| {
        if x < 4 {
            image::Rgba([235, 20, 30, 255])
        } else {
            image::Rgba([20, 30, 235, 255])
        }
    })
    .save(&source)
    .unwrap();
    let source = std::fs::canonicalize(source).unwrap();
    let original = std::fs::read(&source).unwrap();
    let project = directory.join("wand-delete.spectrum");
    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "init",
        "Wand delete CLI",
        "--width",
        "8",
        "--height",
        "4",
        "--background",
        "00000000",
    ]);
    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "add-image",
        source.to_str().unwrap(),
    ]);
    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "selection",
        "magic-wand",
        "1",
        "1",
        "--tolerance",
        "0",
        "--no-antialias",
    ]);
    let deleted = run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "selection",
        "delete",
        "1",
    ]);
    assert_eq!(deleted["results"][0]["action"], "hide_selection");
    assert_eq!(std::fs::read(&source).unwrap(), original);

    let listed = run_canvas(&["--document", project.to_str().unwrap(), "inspect"]);
    let mask = &listed["document"]["layers"][0]["pixel_mask"];
    assert_eq!(mask["width"], 8);
    assert_eq!(mask["height"], 4);
    assert!(
        mask["alpha"]
            .as_str()
            .is_some_and(|alpha| !alpha.is_empty())
    );
    assert!(listed["document"]["selection"].is_object());

    let reopened = saved(&project);
    let pixel_mask = reopened.layers[0].pixel_mask.as_ref().unwrap();
    assert_eq!((pixel_mask.width, pixel_mask.height), (8, 4));
    assert_eq!(
        pixel_mask.alpha.iter().filter(|alpha| **alpha == 0).count(),
        16
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cli_rasterizes_a_shape_through_the_core_command() {
    let directory =
        std::env::temp_dir().join(format!("canvas-cli-rasterize-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("rasterize.spectrum");
    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "init",
        "Rasterize CLI",
        "--width",
        "640",
        "--height",
        "480",
    ]);
    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "add-rectangle",
        "--width",
        "20",
        "--height",
        "10",
        "--radius",
        "3",
    ]);
    let output = run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "rasterize-shape",
        "1",
        "--scale",
        "4",
    ]);
    assert_eq!(output["results"][0]["action"], "rasterize_shape");
    let listed = run_canvas(&["--document", project.to_str().unwrap(), "inspect"]);
    let layer = &listed["document"]["layers"][0];
    assert_eq!(layer["kind"]["type"], "raster");
    assert_eq!(layer["transform"]["scale_x"], 0.25);
    let path = Path::new(layer["kind"]["path"].as_str().unwrap());
    assert_eq!(image::image_dimensions(path).unwrap(), (80, 40));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cli_exposes_extended_blend_modes_through_core_commands() {
    let directory = std::env::temp_dir().join(format!("canvas-cli-blend-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let project = directory.join("blend.spectrum");
    run_canvas(&["--document", project.to_str().unwrap(), "init", "Blend CLI"]);
    run_canvas(&["--document", project.to_str().unwrap(), "add-rectangle"]);
    let result = run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "blend",
        "1",
        "vivid-light",
    ]);
    assert_eq!(result["results"][0]["action"], "set_blend_mode");
    run_canvas(&["--document", project.to_str().unwrap(), "clip", "1", "true"]);
    let listed = run_canvas(&["--document", project.to_str().unwrap(), "inspect"]);
    assert_eq!(listed["document"]["layers"][0]["blend_mode"], "vivid_light");
    assert_eq!(listed["document"]["layers"][0]["clip_to_below"], true);
    let dissolve = run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "blend",
        "1",
        "dissolve",
        "--seed",
        "305419896",
    ]);
    assert_eq!(dissolve["results"][0]["action"], "set_blend_mode");
    assert_eq!(dissolve["results"][1]["action"], "set_dissolve_seed");
    let listed = run_canvas(&["--document", project.to_str().unwrap(), "inspect"]);
    assert_eq!(listed["document"]["layers"][0]["blend_mode"], "dissolve");
    assert_eq!(listed["document"]["layers"][0]["dissolve_seed"], 305419896);
    run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "blend",
        "1",
        "normal",
    ]);
    let dissolve_without_seed = run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "blend",
        "1",
        "dissolve",
    ]);
    assert_eq!(
        dissolve_without_seed["results"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        dissolve_without_seed["results"][0]["action"],
        "set_blend_mode"
    );
    let listed = run_canvas(&["--document", project.to_str().unwrap(), "inspect"]);
    assert_eq!(listed["document"]["layers"][0]["blend_mode"], "dissolve");
    assert_eq!(listed["document"]["layers"][0]["dissolve_seed"], 305419896);
    let revisions_before_seed_update = revision_count(&project);
    let dissolve_with_new_seed = run_canvas(&[
        "--document",
        project.to_str().unwrap(),
        "blend",
        "1",
        "dissolve",
        "--seed",
        "2271560481",
    ]);
    assert_eq!(
        dissolve_with_new_seed["results"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        dissolve_with_new_seed["results"][0]["action"],
        "set_blend_mode"
    );
    assert_eq!(
        dissolve_with_new_seed["results"][1]["action"],
        "set_dissolve_seed"
    );
    let revisions_after_seed_update = revision_count(&project);
    assert_eq!(
        revisions_after_seed_update,
        revisions_before_seed_update + 1
    );
    let listed = run_canvas(&["--document", project.to_str().unwrap(), "inspect"]);
    assert_eq!(
        listed["document"]["layers"][0]["dissolve_seed"],
        2271560481u64
    );
    std::fs::remove_dir_all(directory).unwrap();
}

/// The canvas as last saved.
fn saved(project: &Path) -> Document {
    serde_json::from_value(
        run_canvas(&["--document", project.to_str().unwrap(), "inspect"])["document"].clone(),
    )
    .unwrap()
}

fn revision_count(project: &Path) -> usize {
    run_canvas(&["--document", project.to_str().unwrap(), "history"])["revisions"]
        .as_array()
        .unwrap()
        .len()
}

/// Runs `spectrum canvas`, where `--document <path>` names a test canvas:
/// `init` creates it as an asset in a library beside the path, and later
/// commands edit that asset.
fn run_canvas(arguments: &[&str]) -> Value {
    let (project, rest) = match arguments {
        ["--document", project, rest @ ..] => (Path::new(project), rest),
        _ => panic!("tests name their canvas with --document"),
    };
    let library = project.with_extension("library");
    let marker = project.with_extension("asset");
    let mut command = ProcessCommand::new(env!("CARGO_BIN_EXE_spectrum"));
    command.arg("--library").arg(&library).arg("canvas");
    let creating = rest.first() == Some(&"init");
    if creating {
        command.arg("new").args(&rest[1..]);
    } else {
        let asset = std::fs::read_to_string(&marker).unwrap();
        command.arg("--asset").arg(asset.trim()).args(rest);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "canvas command failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    if creating {
        std::fs::write(&marker, value["id"].as_str().unwrap()).unwrap();
    }
    value
}
