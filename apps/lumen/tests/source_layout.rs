use std::{fs, path::Path};

const MAX_RUST_LINES: usize = 1_000;

fn rust_sources(directory: &Path, output: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(directory).expect("source directory should be readable") {
        let path = entry.expect("source entry should be readable").path();
        if path.is_dir() {
            rust_sources(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            output.push(path);
        }
    }
}

#[test]
fn lumen_source_files_stay_within_the_maintainability_budget() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&source, &mut files);
    let oversized: Vec<_> = files
        .into_iter()
        .filter_map(|path| {
            let lines = fs::read_to_string(&path).ok()?.lines().count();
            (lines > MAX_RUST_LINES).then_some((path, lines))
        })
        .collect();
    assert!(
        oversized.is_empty(),
        "split Lumen sources that exceed {MAX_RUST_LINES} lines: {oversized:#?}"
    );
}

#[test]
fn revision_graph_and_macos_shortcut_use_the_shared_spectrum_surface() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let history = fs::read_to_string(manifest.join("src/bin/lumen_gui/history.rs"))
        .expect("history source should be readable");
    let app = fs::read_to_string(manifest.join("src/bin/lumen-gui.rs"))
        .expect("app source should be readable");
    assert!(history.contains("spectrum_history_ui"));
    assert!(!history.contains("fn tree_layout("));
    assert!(app.contains("spectrum_history_ui::reserve_history_shortcut()"));
    assert!(app.contains("macos::install_open_document_handler"));
}
