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
fn interactive_gui_uses_only_bounded_document_region_rendering() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![manifest.join("src/bin/prism-gui.rs")];
    rust_sources(&manifest.join("src/bin/prism_gui"), &mut files);
    let offenders: Vec<_> = files
        .into_iter()
        .filter(|path| {
            fs::read_to_string(path).is_ok_and(|source| {
                source.contains("render_document(") || source.contains("render_document_scaled(")
            })
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "interactive Prism code must not allocate full-document previews: {offenders:#?}"
    );
    let compositor = fs::read_to_string(manifest.join("src/bin/prism_gui/compositor.rs"))
        .expect("region compositor should be readable");
    assert!(compositor.contains("render_document_region_scaled("));
}

#[test]
fn continuous_inspector_controls_use_gesture_transactions() {
    let inspector = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/prism_gui/inspector.rs"),
    )
    .expect("inspector source should be readable");
    assert!(
        !inspector.contains("self.execute(Command::UpdateRectangle"),
        "rectangle sliders must preview one transaction, not commit every rendered frame"
    );
    assert!(
        !inspector.contains("self.execute(Command::UpdateText"),
        "text editing must preview one transaction, not commit every keystroke"
    );
}

#[test]
fn inline_text_editor_uses_one_preview_transaction_and_keeps_add_text_dialog() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let inline = fs::read_to_string(manifest.join("src/bin/prism_gui/inline_text.rs"))
        .expect("inline text editor source should be readable");
    let dialogs = fs::read_to_string(manifest.join("src/bin/prism_gui/dialogs.rs"))
        .expect("dialog source should be readable");
    assert!(inline.contains("self.workspace.begin_interaction()"));
    assert!(inline.contains("self.preview_command(command)"));
    assert!(inline.contains("self.finish_interaction()"));
    assert!(inline.contains("self.workspace.cancel_interaction()"));
    assert!(inline.contains("self.layer_visual_dirty.insert(layer_id)"));
    assert!(!inline.contains("self.execute(Command::UpdateText"));
    assert!(dialogs.contains("self.text_dialog = Some(TextDialogDraft"));
    assert!(dialogs.contains("Command::AddText"));
}

#[test]
fn revision_graph_comes_from_the_shared_spectrum_surface() {
    let history = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/prism_gui/history.rs"),
    )
    .expect("history source should be readable");
    assert!(history.contains("spectrum_history_ui"));
    assert!(!history.contains("fn tree_layout("));
}
