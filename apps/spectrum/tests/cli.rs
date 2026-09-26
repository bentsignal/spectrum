use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};

fn call(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_spectrum"))
        .arg("--library")
        .arg(root)
        .args(args)
        .env_remove("SPECTRUM_IMAGES_DOCUMENT")
        .env_remove("SPECTRUM_CANVAS_DOCUMENT")
        .env_remove("SPECTRUM_SESSION")
        .env_remove("SPECTRUM_LIVE_MODE")
        .output()
        .unwrap()
}
fn ok(root: &Path, args: &[&str]) -> Value {
    let result = call(root, args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

#[test]
fn help_and_schema_cover_both_engines_without_creating_library() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("absent");
    for args in [
        vec!["--help"],
        vec!["images", "--help"],
        vec!["canvas", "--help"],
    ] {
        assert!(call(&root, &args).status.success());
    }
    let schema = ok(&root, &["schema"]);
    assert!(schema["images"].is_object());
    assert!(schema["canvas"].is_object());
    for domain in ["images", "canvas"] {
        assert!(ok(&root, &[domain, "schema"]).is_object());
        let output = call(&root, &[domain, "inspect"]);
        assert!(!output.status.success());
        assert!(
            serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("select a target")
        );
    }
    assert!(!root.exists());
}

#[test]
fn consolidated_commands_edit_export_and_keep_linked_assets_current() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let source = temp.path().join("original.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba([50, 60, 70, 255]))
        .save(&source)
        .unwrap();
    let images = ok(&root, &["images", "import", source.to_str().unwrap()]);
    let image = images[0]["id"].as_str().unwrap();
    let item = images[0]["item"].as_u64().unwrap().to_string();
    let canvas = ok(
        &root,
        &["canvas", "new", "Test", "--width", "16", "--height", "16"],
    );
    let canvas = canvas["id"].as_str().unwrap();
    ok(&root, &["canvas", "place", canvas, image]);
    ok(
        &root,
        &["images", "--asset", image, "edit", &item, "--exposure", "1"],
    );
    let inspected = ok(&root, &["images", "--asset", image, "get", &item]);
    assert!(inspected.to_string().contains("exposure"));
    ok(&root, &["images", "--asset", image, "history", &item]);
    ok(&root, &["canvas", "--asset", canvas, "add-text", "Hello"]);
    let document = ok(&root, &["canvas", "--asset", canvas, "inspect"]);
    assert_eq!(document["document"]["layers"].as_array().unwrap().len(), 2);
    let export = temp.path().join("export.jpg");
    ok(
        &root,
        &[
            "images",
            "export",
            image,
            export.to_str().unwrap(),
            "--quality",
            "80",
            "--max-size",
            "8",
        ],
    );
    assert_eq!(image::image_dimensions(&export).unwrap(), (8, 8));
    ok(
        &root,
        &[
            "canvas",
            "export",
            canvas,
            temp.path().join("canvas.png").to_str().unwrap(),
        ],
    );
    let wrong = call(&root, &["canvas", "--asset", image, "inspect"]);
    assert!(!wrong.status.success());
    let missing = call(&root, &["images", "--asset", canvas, "inspect"]);
    assert!(!missing.status.success());
    let forbidden = call(
        &root,
        &[
            "images",
            "export",
            image,
            root.join("overwrite.png").to_str().unwrap(),
        ],
    );
    assert!(!forbidden.status.success());
    // Asset targeting must override an inherited terminal document, never edit it accidentally.
    let result = Command::new(env!("CARGO_BIN_EXE_spectrum"))
        .arg("--library")
        .arg(&root)
        .args(["images", "--asset", image, "get", &item])
        .env("SPECTRUM_IMAGES_DOCUMENT", "/nonexistent/other-document")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
