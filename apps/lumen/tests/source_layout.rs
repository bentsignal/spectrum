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
fn revision_graph_and_macos_menu_use_the_shared_spectrum_surface() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let history = fs::read_to_string(manifest.join("src/bin/lumen_gui/history.rs"))
        .expect("history source should be readable");
    let app = fs::read_to_string(manifest.join("src/bin/lumen-gui.rs"))
        .expect("app source should be readable");
    assert!(history.contains("spectrum_history_ui"));
    assert!(!history.contains("fn tree_layout("));
    assert!(app.contains("spectrum_history_ui::reserve_history_shortcut()"));
    assert!(app.contains("macos::run(initial_catalog)"));
}

#[test]
fn native_macos_menu_owns_catalog_photo_and_edit_navigation() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let macos = fs::read_to_string(manifest.join("src/bin/lumen_gui/macos.rs"))
        .expect("macOS integration source should be readable");
    let spec = fs::read_to_string(manifest.join("src/bin/lumen_gui/macos_menu_spec.rs"))
        .expect("macOS menu specification should be readable");
    let toolbar = fs::read_to_string(manifest.join("src/bin/lumen_gui/toolbar.rs"))
        .expect("toolbar source should be readable");

    let disable_default = macos
        .find("event_loop_builder.with_default_menu(false)")
        .expect("Lumen must own the native macOS menu");
    let build = macos
        .find("event_loop_builder.build()")
        .expect("the explicit event loop must be retained");
    let install = macos
        .find("install_app_integration(open_document_sender, native_menu_sender)")
        .expect("menu integration must be installed before app creation");
    assert!(disable_default < build && build < install);
    for action in [
        "ImportPhotos",
        "ExportPhotos",
        "ToggleWorkspaceView",
        "PreviousPhoto",
        "NextPhoto",
        "Undo",
        "Redo",
    ] {
        assert!(spec.contains(action));
    }
    assert!(toolbar.contains("#[cfg(not(target_os = \"macos\"))]"));
}

#[test]
fn routine_status_and_pick_glyph_chatter_stay_out_of_lumen_ui() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dialogs = fs::read_to_string(manifest.join("src/bin/lumen_gui/dialogs.rs")).unwrap();
    let toolbar = fs::read_to_string(manifest.join("src/bin/lumen_gui/toolbar.rs")).unwrap();
    let library = fs::read_to_string(manifest.join("src/bin/lumen_gui/library.rs")).unwrap();
    assert!(dialogs.contains("if self.error"));
    assert!(!toolbar.contains("selectable_label(true, \"+\")"));
    assert!(!toolbar.contains("selectable_label(true, \"x\")"));
    assert!(!library.contains("RichText::new(\"+\")"));
    assert!(!library.contains("RichText::new(\"x\")"));
    assert!(!library.contains("Back to Photos"));
}

#[test]
fn catalog_navigation_has_one_top_level_switch_and_two_restrained_primary_actions() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let toolbar = fs::read_to_string(manifest.join("src/bin/lumen_gui/toolbar.rs"))
        .expect("toolbar source should be readable");
    let library = fs::read_to_string(manifest.join("src/bin/lumen_gui/library.rs"))
        .expect("catalog source should be readable");

    let wordmark = toolbar.find("\"LUMEN\"").expect("wordmark should remain");
    let view_switch = toolbar[wordmark..]
        .find("view_switch_presentation(")
        .map(|offset| wordmark + offset)
        .expect("workspace switch should follow the wordmark");
    let terminal = toolbar[view_switch..]
        .find("if self.terminal.visible()")
        .map(|offset| view_switch + offset)
        .expect("remaining toolbar actions should follow the workspace switch");
    assert!(wordmark < view_switch && view_switch < terminal);
    assert!(!toolbar.contains("divider_rect"));
    assert!(toolbar.contains("ui.separator()"));
    assert!(!toolbar.contains("button(\"All Shoots\")"));
    assert!(!toolbar.contains("RichText::new(\"ALL SHOOTS\")"));

    assert!(library.contains(".button(\"New Shoot\")"));
    assert!(library.contains("self.new_catalog()"));
    assert!(library.contains(".button(\"Import Photos\")"));
    assert!(library.contains("self.import_dialog()"));
    assert!(library.contains("Vec2::new(ui.available_width(), 44.0)"));
    assert!(library.contains("Some(*photo_id)"));
    assert!(!library.contains("photo_view_return_label"));
}

#[test]
fn catalog_shortcut_labels_are_consistent_across_native_and_portable_surfaces() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let toolbar = fs::read_to_string(manifest.join("src/bin/lumen_gui/toolbar.rs"))
        .expect("toolbar source should be readable");
    let menu = fs::read_to_string(manifest.join("src/bin/lumen_gui/macos_menu_spec.rs"))
        .expect("native menu source should be readable");

    assert!(toolbar.contains("Move Catalog...  Ctrl+Shift+M"));
    assert!(menu.contains("Some(ActionKeyEquivalent::command(\"i\"))"));
    assert!(menu.contains("Some(ActionKeyEquivalent::command_shift(\"m\"))"));
}

#[test]
fn catalog_and_filmstrip_thumbnails_stay_on_the_display_only_proxy_path() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let state = fs::read_to_string(manifest.join("src/bin/lumen_gui/state.rs"))
        .expect("Lumen GUI state source should be readable");
    let start = state
        .find("pub(super) fn ensure_thumbnail")
        .expect("thumbnail renderer should remain explicit");
    let end = state[start..]
        .find("pub(super) fn handle_drop_and_shortcuts")
        .map(|offset| start + offset)
        .expect("thumbnail renderer should remain bounded");
    let thumbnail_renderer = &state[start..end];

    assert!(thumbnail_renderer.contains("decode_photo_proxy(photo, 240)"));
    assert!(!thumbnail_renderer.contains("render_settled_preview"));
    assert!(!thumbnail_renderer.contains("render_photo("));
}

#[test]
fn lumen_branding_is_wired_to_runtime_and_native_packages() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = manifest.join("../..");
    let app = fs::read_to_string(manifest.join("src/bin/lumen-gui.rs")).unwrap();
    let plist = fs::read_to_string(repository.join("packaging/macos/Info.plist")).unwrap();
    let macos = fs::read_to_string(repository.join("scripts/package-macos.sh")).unwrap();
    let linux = fs::read_to_string(repository.join("scripts/package-linux.sh")).unwrap();
    let windows = fs::read_to_string(repository.join("scripts/package-windows.ps1")).unwrap();

    assert!(app.contains("with_icon(lumen_icon())"));
    assert!(app.contains("lumen-app-icon.png"));
    assert!(plist.contains("<string>Lumen.icns</string>"));
    assert!(macos.contains("lumen-app-icon.png"));
    assert!(linux.contains("com.bentsignal.Lumen.png"));
    assert!(windows.contains("Lumen.png"));
}
