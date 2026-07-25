use super::*;

pub(super) fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 940.0])
            .with_min_inner_size([980.0, 640.0])
            .with_icon(prism_icon()),
        centered: true,
        ..Default::default()
    }
}

pub(super) fn prism_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../../../assets/branding/prism-app-icon.png"
    ))
    .expect("bundled Prism icon must be a valid PNG")
}

#[cfg(not(target_os = "macos"))]
pub(super) fn run() -> eframe::Result {
    let initial_project = std::env::args_os().nth(1).map(PathBuf::from);
    let (_, open_document_receiver) = mpsc::channel();
    eframe::run_native(
        "Prism",
        native_options(),
        Box::new(move |creation| {
            Ok(Box::new(PrismApp::new(
                creation,
                initial_project.as_deref(),
                open_document_receiver,
            )))
        }),
    )
}

#[cfg(target_os = "macos")]
pub(super) fn run() -> eframe::Result {
    let initial_project = std::env::args_os().nth(1).map(PathBuf::from);
    macos::run(initial_project)
}
