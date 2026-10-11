//! History commands for images and canvases: naming revisions, searching,
//! previews of any revision, and going to where another session is.
use serde_json::Value;
use std::{path::Path, process::Command};

fn ok(root: &Path, args: &[&str]) -> Value {
    let result = Command::new(env!("CARGO_BIN_EXE_spectrum"))
        .arg("--library")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

fn fails(root: &Path, args: &[&str]) {
    let result = Command::new(env!("CARGO_BIN_EXE_spectrum"))
        .arg("--library")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(!result.status.success(), "{args:?} should fail");
}

/// Names, search, previews, and jumps on one asset of either kind, after
/// `edit` has made two edits on it.
fn check(root: &Path, domain: &str, asset: &str, edit: &[&str]) {
    let history = ok(root, &[domain, "--asset", asset, "history"]);
    let revisions = history["revisions"].as_array().unwrap().clone();
    assert_eq!(revisions.len(), 3, "{history}");
    assert!(history["names"].as_object().unwrap().is_empty());
    let first = revisions[1]["id"].as_str().unwrap().to_string();
    // The first line of work begins at the first revision, so naming any
    // revision on it names the branch there.
    let start = revisions[0]["id"].as_str().unwrap().to_string();
    let current = history["current"].as_str().unwrap().to_string();

    // A name is not an edit: no revision is added and no one moves.
    ok(
        root,
        &[
            domain,
            "--asset",
            asset,
            "history-name",
            &first,
            "Warm look",
        ],
    );
    let named = ok(root, &[domain, "--asset", asset, "history"]);
    assert_eq!(named["revisions"].as_array().unwrap().len(), 3);
    assert_eq!(named["current"], current.as_str());
    assert_eq!(named["names"][&start], "Warm look");

    // Search finds names, ignoring case, and edit descriptions.
    let found = ok(
        root,
        &[domain, "--asset", asset, "history", "--search", "warm"],
    );
    assert_eq!(found["matches"], serde_json::json!([start]));
    let nothing = ok(
        root,
        &[domain, "--asset", asset, "history", "--search", "no such"],
    );
    assert_eq!(nothing["matches"], serde_json::json!([]));

    // Renaming replaces; clearing removes.
    ok(
        root,
        &[domain, "--asset", asset, "history-name", &first, "Final"],
    );
    let renamed = ok(root, &[domain, "--asset", asset, "history"]);
    assert_eq!(renamed["names"][&start], "Final");
    ok(
        root,
        &[domain, "--asset", asset, "history-name", &first, "--clear"],
    );
    let cleared = ok(root, &[domain, "--asset", asset, "history"]);
    assert!(cleared["names"].as_object().unwrap().is_empty());
    fails(root, &[domain, "--asset", asset, "history-name", &first]);

    // A preview of an earlier revision, made without moving anyone.
    let preview = ok(
        root,
        &[
            domain,
            "--asset",
            asset,
            "history-preview",
            &first,
            "--size",
            "32",
        ],
    );
    let path = preview["path"].as_str().unwrap();
    let image = image::open(path).unwrap();
    assert!(image.width() <= 32 && image.height() <= 32);
    let after = ok(root, &[domain, "--asset", asset, "history"]);
    assert_eq!(after["current"], current.as_str());

    // An agent's separate session, then the person going to where it is.
    let started = ok(
        root,
        &[
            "agent", "start", asset, "--mode", "separate", "--name", "Helper",
        ],
    );
    let session = started["session"].as_str().unwrap().to_string();
    ok(root, &[domain, "--asset", asset, "history-jump", &first]);
    let moved = ok(root, &[domain, "--asset", asset, "history"]);
    assert_eq!(moved["current"], first.as_str());

    // Editing there branches; naming the new branch names only it.
    let mut args = vec![domain, "--asset", asset];
    args.extend_from_slice(edit);
    ok(root, &args);
    let branched = ok(root, &[domain, "--asset", asset, "history"]);
    let side = branched["current"].as_str().unwrap().to_string();
    ok(
        root,
        &[domain, "--asset", asset, "history-name", &side, "Side"],
    );
    ok(
        root,
        &[domain, "--asset", asset, "history-name", &first, "Main"],
    );
    let both = ok(root, &[domain, "--asset", asset, "history"]);
    assert_eq!(both["names"][&side], "Side");
    assert_eq!(both["names"][&start], "Main");
    assert_eq!(both["names"].as_object().unwrap().len(), 2);
    ok(root, &[domain, "--asset", asset, "history-jump", &first]);
    ok(
        root,
        &[
            domain,
            "--asset",
            asset,
            "history-jump",
            "--to-session",
            &session,
        ],
    );
    let joined = ok(root, &[domain, "--asset", asset, "history"]);
    let agent_place = joined["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == session.as_str())
        .unwrap()["cursor"]
        .clone();
    assert_eq!(joined["current"], agent_place);
}

#[test]
fn image_and_canvas_revisions_are_named_searched_previewed_and_joined() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let source = temp.path().join("photo.png");
    image::RgbaImage::from_pixel(48, 32, image::Rgba([90, 120, 150, 255]))
        .save(&source)
        .unwrap();
    let imported = ok(&root, &["images", "import", source.to_str().unwrap()]);
    let image = imported["assets"][0]["id"].as_str().unwrap().to_string();
    ok(
        &root,
        &["images", "--asset", &image, "edit", "--exposure", "1"],
    );
    ok(
        &root,
        &["images", "--asset", &image, "edit", "--contrast", "20"],
    );
    check(&root, "images", &image, &["edit", "--exposure", "2"]);

    let canvas = ok(
        &root,
        &["canvas", "new", "Board", "--width", "64", "--height", "48"],
    );
    let canvas = canvas["id"].as_str().unwrap().to_string();
    ok(&root, &["canvas", "--asset", &canvas, "add-text", "Hello"]);
    ok(&root, &["canvas", "--asset", &canvas, "add-text", "Again"]);
    check(&root, "canvas", &canvas, &["add-text", "Branch"]);
}
