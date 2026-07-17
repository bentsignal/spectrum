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
fn prism_source_files_stay_within_the_maintainability_budget() {
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
        "split Prism sources that exceed {MAX_RUST_LINES} lines: {oversized:#?}"
    );
}

#[test]
fn interactive_gui_never_calls_the_full_document_compositor() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![manifest.join("src/bin/prism-gui.rs")];
    rust_sources(&manifest.join("src/bin/prism_gui"), &mut files);
    let offenders: Vec<_> = files
        .into_iter()
        .filter(|path| {
            fs::read_to_string(path).is_ok_and(|source| source.contains("render_document("))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "interactive Prism code must render cached layers, not full documents: {offenders:#?}"
    );
}
