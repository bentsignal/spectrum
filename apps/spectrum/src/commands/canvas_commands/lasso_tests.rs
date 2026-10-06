use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_project(label: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("canvas-cli-{label}-{timestamp}.spectrum"))
}

#[test]
fn lasso_cli_persists_fixed_point_soft_selection_and_rejects_short_paths() {
    let project = temporary_project("selection-lasso");
    let project_arg = project.to_str().unwrap();
    run(parse_cli([
        "canvas",
        "--document",
        project_arg,
        "init",
        "Lasso CLI",
        "--width",
        "64",
        "--height",
        "64",
    ])
    .unwrap())
    .unwrap();
    assert!(
        run(parse_cli([
            "canvas",
            "--document",
            project_arg,
            "selection",
            "lasso",
            "--point",
            "4,5",
            "--point",
            "40,6",
        ])
        .unwrap())
        .is_err()
    );
    assert!(Workspace::read(&project).unwrap().selection.is_none());
    run(parse_cli([
        "canvas",
        "--document",
        project_arg,
        "selection",
        "lasso",
        "--point",
        "4.25,5.5",
        "--point",
        "40.5,6.25",
        "--point",
        "8.75,42.5",
        "--mode",
        "replace",
    ])
    .unwrap())
    .unwrap();
    let document = Workspace::read(&project).unwrap();
    let selection = document.selection.unwrap();
    assert!(selection.alpha().is_some());
    assert!(selection.bounds().2 < document.width && selection.bounds().3 < document.height);
    std::fs::remove_file(project).unwrap();
}
