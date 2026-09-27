use serde_json::Value;
use std::{path::Path, process::Command};

fn ok(root: &Path, args: &[&str]) -> Value {
    let result = Command::new(env!("CARGO_BIN_EXE_spectrum"))
        .arg("--library")
        .arg(root)
        .args(args)
        .env_remove("SPECTRUM_IMAGES_DOCUMENT")
        .env_remove("SPECTRUM_CANVAS_DOCUMENT")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

fn center(path: &Path) -> [u8; 4] {
    let image = image::open(path).unwrap().to_rgba8();
    image.get_pixel(20, 15).0
}

#[test]
fn deleted_images_wait_in_trash_and_canvases_draw_placeholders() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let source = temp.path().join("red.png");
    image::RgbaImage::from_pixel(40, 30, image::Rgba([255, 0, 0, 255]))
        .save(&source)
        .unwrap();
    let imported = ok(
        &root,
        &[
            "images",
            "import",
            source.to_str().unwrap(),
            "--new-project",
            "Trip",
        ],
    );
    let image = imported["assets"][0]["id"].as_str().unwrap().to_string();
    let project = imported["project"].as_str().unwrap().to_string();
    let canvas = ok(
        &root,
        &["canvas", "new", "Poster", "--width", "40", "--height", "30"],
    );
    let canvas = canvas["id"].as_str().unwrap().to_string();
    ok(&root, &["canvas", "place", &canvas, &image]);
    let export = |name: &str| {
        let path = temp.path().join(name);
        ok(
            &root,
            &["canvas", "export", &canvas, path.to_str().unwrap()],
        );
        center(&path)
    };
    assert_eq!(export("before.png"), [255, 0, 0, 255]);

    let deleted = ok(&root, &["delete", &image]);
    assert_eq!(deleted["projects"][0]["name"], "Trip");
    assert_eq!(deleted["dependents"][0]["id"], canvas.as_str());
    let placeholder = export("trashed.png");
    assert_ne!(placeholder, [255, 0, 0, 255]);
    assert_eq!(
        ok(&root, &["projects", "show", &project])["assets"],
        Value::Array(vec![])
    );
    assert_eq!(ok(&root, &["trash", "list"]).as_array().unwrap().len(), 1);

    ok(&root, &["trash", "restore", &image]);
    assert_eq!(export("restored.png"), [255, 0, 0, 255]);
    assert_eq!(
        ok(&root, &["projects", "show", &project])["project"]["assets"],
        1
    );

    ok(&root, &["delete", &image]);
    let document = root.join(imported["assets"][0]["document"].as_str().unwrap());
    assert!(document.exists());
    ok(&root, &["trash", "empty"]);
    assert!(!document.exists());
    assert!(ok(&root, &["trash", "list"]).as_array().unwrap().is_empty());
    assert_eq!(export("purged.png"), placeholder);
    let copy = ok(&root, &["copy", &canvas]);
    assert_eq!(copy["kind"], "canvas");
}
