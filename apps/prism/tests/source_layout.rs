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
fn font_subset_plan_preview_is_button_gated_revision_cached_and_explicitly_read_only() {
    let typography = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/prism_gui/typography_ui.rs"),
    )
    .expect("typography inspector source should be readable");
    let start = typography.find("fn font_usage_controls(").unwrap();
    let end = typography[start..]
        .find("pub(super) fn paragraph_controls(")
        .map(|offset| start + offset)
        .unwrap();
    let controls = &typography[start..end];
    let button = controls
        .find("small_button(\"Preview optimized-copy savings\").clicked()")
        .unwrap();
    let plan = controls.find("plan_font_subset(").unwrap();
    assert!(button < plan);
    assert_eq!(controls.matches("plan_font_subset(").count(), 1);
    assert!(!controls.contains("self.execute("));
    assert!(controls.contains("editable project unchanged"));
    assert!(controls.contains("no smaller copy created"));
    assert!(controls.contains("history-safe compact-copy export"));
    for stable_key_part in [
        "active_tab_id",
        "document_identity",
        "document_generation",
        "content_hash",
    ] {
        assert!(typography.contains(stable_key_part));
    }
}

#[test]
fn single_command_previews_do_not_clone_the_document_per_frame() {
    let workspace =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/workspace.rs"))
            .expect("workspace source should be readable");
    let preview_start = workspace.find("pub fn preview(").unwrap();
    let batch_start = workspace.find("pub fn preview_batch(").unwrap();
    let single_preview = &workspace[preview_start..batch_start];
    assert!(single_preview.contains("apply_command(&mut self.document"));
    assert!(!single_preview.contains("self.document.clone()"));
    let commit_start = workspace.find("pub fn commit_interaction(").unwrap();
    let batch_preview = &workspace[batch_start..commit_start];
    assert!(
        batch_preview.find("commands.len() == 1").unwrap()
            < batch_preview.find("self.document.clone()").unwrap()
    );
}

#[test]
fn inline_text_editor_owns_existing_edits_and_click_to_type_creation() {
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
    assert!(inline.contains("let source_geometry = self.layer_source_geometry(layer);"));
    assert!(inline.contains("transformed_visual_screen_bounds(geometry, layer, source_geometry)"));
    assert!(!inline.contains("self.execute(Command::UpdateText"));
    assert!(inline.contains("Command::AddText"));
    assert!(inline.contains("open_new_text_editor"));
    assert!(inline.contains("editor_visual_screen_bounds(geometry, &editor, rendered_bounds)"));
    assert!(inline.contains("let area_id = editor.area_id();"));
    assert!(!inline.contains("editor.tab_id, editor.layer_id"));
    assert!(!dialogs.contains("TextDialogDraft"));
    assert!(!dialogs.contains("Command::AddText"));
}

#[test]
fn layer_transfer_core_and_cli_stay_in_dedicated_modules() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let library = fs::read_to_string(manifest.join("src/lib.rs")).unwrap();
    let commands = fs::read_to_string(manifest.join("src/commands.rs")).unwrap();
    let core = fs::read_to_string(manifest.join("src/transfer.rs")).unwrap();
    let cli = fs::read_to_string(manifest.join("src/bin/prism_cli/transfer.rs")).unwrap();
    let binary = fs::read_to_string(manifest.join("src/bin/prism.rs")).unwrap();

    assert!(library.contains("mod transfer;"));
    assert!(commands.contains("InsertLayer"));
    assert!(commands.contains("transfer: Box<LayerTransfer>"));
    assert!(core.contains("LAYER_TRANSFER_VERSION"));
    assert!(core.contains("document-local layer ID"));
    assert!(core.contains("document-local font ID"));
    assert!(cli.contains("LayerCopyArgs"));
    assert!(cli.contains("LayerPasteArgs"));
    assert!(binary.contains("prism_cli/transfer.rs"));
    assert!(cli.lines().count() < 200);
}

#[test]
fn prism_cli_delegates_agent_collaboration_with_binary_headroom() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let binary = fs::read_to_string(manifest.join("src/bin/prism.rs")).unwrap();
    let agent = fs::read_to_string(manifest.join("src/bin/prism_cli/agent.rs")).unwrap();

    assert!(binary.contains("prism_cli/agent.rs"));
    assert!(!binary.contains("fn agent_command("));
    assert!(agent.contains("pub(super) fn agent_command("));
    assert!(agent.contains("fn local_gui_session_id("));
    assert!(binary.lines().count() < 950);
}

#[test]
fn prism_schema_builds_command_examples_without_macro_recursion_overrides() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let schema = fs::read_to_string(manifest.join("src/bin/prism_cli/schema.rs")).unwrap();
    let binary = fs::read_to_string(manifest.join("src/bin/prism.rs")).unwrap();

    assert!(schema.contains("let command_examples = command_examples();"));
    assert!(schema.contains("\"examples\": command_examples"));
    assert!(schema.contains("fn command_examples() -> Vec<Value>"));
    assert!(!schema.contains("recursion_limit"));
    assert!(!binary.contains("recursion_limit"));
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

#[test]
fn native_macos_menu_disables_winit_replacement_before_launch() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(manifest.join("src/bin/prism_gui/macos.rs"))
        .expect("macOS integration source should be readable");
    let disable_default = source
        .find("event_loop_builder.with_default_menu(false)")
        .expect("Prism must own the process-wide macOS menu");
    let build = source
        .find("event_loop_builder.build()")
        .expect("event loop builder should remain explicit");
    let install = source
        .find("install_app_integration(open_document_sender, native_menu_sender)")
        .expect("Prism menu should be installed after creating the event loop");
    assert!(disable_default < build && build < install);

    let binary = fs::read_to_string(manifest.join("src/bin/prism-gui.rs"))
        .expect("Prism GUI entry point should be readable");
    assert!(!binary.contains("with_default_menu("));
    assert!(binary.contains("#[cfg(not(target_os = \"macos\"))]\nfn main()"));
    assert!(binary.contains("eframe::run_native("));
}

#[test]
fn prism_branding_uses_the_user_crop_in_runtime_and_native_packages() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = manifest.join("../..");
    let app = fs::read_to_string(manifest.join("src/bin/prism-gui.rs")).unwrap();
    let plist = fs::read_to_string(repository.join("packaging/prism/macos/Info.plist")).unwrap();
    let macos = fs::read_to_string(repository.join("scripts/package-prism-macos.sh")).unwrap();
    let linux = fs::read_to_string(repository.join("scripts/package-prism-linux.sh")).unwrap();
    let windows = fs::read_to_string(repository.join("scripts/package-prism-windows.ps1")).unwrap();

    assert!(app.contains("with_icon(prism_icon())"));
    assert!(app.contains("assets/branding/prism-app-icon.png"));
    assert!(plist.contains("<string>Prism.icns</string>"));
    assert!(plist.contains("<key>CFBundleIconName</key>"));
    assert!(plist.contains("<string>Prism</string>"));
    assert!(macos.contains("assets/branding/Prism.icon"));
    assert!(linux.contains("com.bentsignal.Prism.png"));
    assert!(windows.contains("Prism.png"));

    let native_icon = repository.join("assets/branding/Prism.icon");
    let icon_source = fs::read_to_string(native_icon.join("icon.json")).unwrap();
    let icon: serde_json::Value = serde_json::from_str(&icon_source).unwrap();
    let source = fs::read(repository.join("assets/branding/cropped-prism.png")).unwrap();
    let embedded = fs::read(native_icon.join("Assets/cropped-prism.png")).unwrap();
    assert_eq!(
        source, embedded,
        "Icon Composer must preserve the approved crop"
    );
    assert_eq!(icon["supported-platforms"]["squares"], "shared");
    let group = &icon["groups"][0];
    assert!(
        group.get("position").is_none(),
        "Prism scale belongs on each 400-pixel artwork layer, never the group"
    );
    let layers = group["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0]["image-name"], "cropped-prism.png");
    assert_eq!(layers[1]["image-name"], "prism-mono.png");
    assert!(
        layers
            .iter()
            .all(|layer| layer["position"]["scale"] == 2.56)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn prism_macos_package_build_is_bash_3_safe_and_preserves_cargo_failure() {
    use std::{
        os::unix::fs::PermissionsExt,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    let repository = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("repository root should be canonicalizable");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let fixture = std::env::temp_dir().join(format!(
        "prism-package-bash3-{}-{stamp}",
        std::process::id()
    ));
    let shims = fixture.join("shims");
    fs::create_dir_all(&shims).unwrap();
    let cargo_log = fixture.join("cargo-arguments");
    let cargo = shims.join("cargo");
    fs::write(
        &cargo,
        "#!/bin/bash\nprintf '%s\\n' \"$@\" >\"$PRISM_PACKAGE_CARGO_LOG\"\nexit 47\n",
    )
    .unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new("/bin/bash")
        .arg(repository.join("scripts/package-prism-macos.sh"))
        .current_dir(&repository)
        .env("PATH", format!("{}:/usr/bin:/bin", shims.display()))
        .env("PRISM_PACKAGE_CARGO_LOG", &cargo_log)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(47), "{output:#?}");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("unbound variable"));
    assert_eq!(
        fs::read_to_string(&cargo_log).unwrap(),
        "build\n--release\n--locked\n-p\nprism\n--bins\n"
    );

    let package_source = fs::read_to_string(repository.join("scripts/package-prism-macos.sh"))
        .expect("Prism package script should be readable");
    let cleanup_start = package_source
        .find("cleanup_private_root() {")
        .expect("package script should define cleanup_private_root");
    let cleanup_end = package_source[cleanup_start..]
        .find("\n}\ntrap cleanup_private_root EXIT")
        .map(|offset| cleanup_start + offset + 2)
        .expect("cleanup function should precede its EXIT trap");
    let private_root = repository.join(format!("target/spectrum-ghostty-package.test-{stamp}"));
    fs::create_dir_all(private_root.join("proof")).unwrap();
    let cleanup_harness = fixture.join("cleanup-harness.sh");
    fs::write(
        &cleanup_harness,
        format!(
            "#!/bin/bash\nset -euo pipefail\nrepo_root=\"$PRISM_TEST_REPO_ROOT\"\nprivate_root=\"$PRISM_TEST_PRIVATE_ROOT\"\n{}\ntrap cleanup_private_root EXIT\nexit 53\n",
            &package_source[cleanup_start..cleanup_end]
        ),
    )
    .unwrap();
    let cleanup_status = Command::new("/bin/bash")
        .arg(&cleanup_harness)
        .env("PRISM_TEST_REPO_ROOT", &repository)
        .env("PRISM_TEST_PRIVATE_ROOT", &private_root)
        .status()
        .unwrap();
    assert_eq!(cleanup_status.code(), Some(53));
    assert!(!private_root.exists());
    fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn lone_document_tab_close_is_disabled_and_annotated() {
    let chrome = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/prism_gui/chrome.rs"),
    )
    .expect("Prism chrome source should be readable");
    let compact_chrome = chrome.split_whitespace().collect::<String>();
    assert!(compact_chrome.contains(".add_enabled(close_affordance.enabled,"));
    assert!(chrome.contains("on_disabled_hover_text(close_affordance.hover_text)"));
}
