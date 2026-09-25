#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
};

use eframe::egui;

#[allow(dead_code)]
#[path = "../../lumen/src/bin/lumen-gui.rs"]
mod lumen_gui;
#[cfg(target_os = "macos")]
mod macos;
mod perf;
#[allow(dead_code)]
#[path = "../../prism/src/bin/prism-gui.rs"]
mod prism_gui;

const ACTIVE_WORKSPACE_KEY: &str = "spectrum-active-workspace-v1";

fn spectrum_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../assets/branding/prism-app-icon.png"
    ))
    .expect("bundled Spectrum icon must be a valid PNG")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceKind {
    Photo,
    Canvas,
}

impl WorkspaceKind {
    fn from_path(path: &std::path::Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "lumen" | "lumencatalog" => Some(Self::Photo),
            "prism" | "mica" => Some(Self::Canvas),
            _ => None,
        }
    }

    fn storage_value(self) -> &'static str {
        match self {
            Self::Photo => "photo",
            Self::Canvas => "canvas",
        }
    }
}

struct SpectrumApp {
    photo: lumen_gui::LumenApp,
    canvas: prism_gui::PrismApp,
    active: WorkspaceKind,
    photo_style: egui::Style,
    canvas_style: egui::Style,
    open_document_receiver: Receiver<PathBuf>,
    photo_document_sender: Sender<PathBuf>,
    canvas_document_sender: Sender<PathBuf>,
    trace: Option<perf::FrameTrace>,
}

impl SpectrumApp {
    fn new(
        creation: &eframe::CreationContext<'_>,
        startup_path: Option<PathBuf>,
        open_document_receiver: Receiver<PathBuf>,
    ) -> Self {
        let path_kind = startup_path.as_deref().and_then(WorkspaceKind::from_path);
        let requested = path_kind
            .or_else(|| {
                creation
                    .storage
                    .and_then(|storage| storage.get_string(ACTIVE_WORKSPACE_KEY))
                    .and_then(|value| match value.as_str() {
                        "photo" => Some(WorkspaceKind::Photo),
                        "canvas" => Some(WorkspaceKind::Canvas),
                        _ => None,
                    })
            })
            .unwrap_or(WorkspaceKind::Photo);
        let photo_path = (path_kind == Some(WorkspaceKind::Photo))
            .then(|| startup_path.clone())
            .flatten();
        let canvas_path = (path_kind == Some(WorkspaceKind::Canvas))
            .then_some(startup_path.as_deref())
            .flatten();
        let (photo_document_sender, photo_receiver) = mpsc::channel();
        let photo = lumen_gui::LumenApp::for_spectrum(creation, photo_path, photo_receiver);
        let photo_style = (*creation.egui_ctx.style_of(egui::Theme::Dark)).clone();
        let (canvas_document_sender, canvas_receiver) = mpsc::channel();
        let canvas = prism_gui::PrismApp::for_spectrum(creation, canvas_path, canvas_receiver);
        let canvas_style = (*creation.egui_ctx.style_of(egui::Theme::Dark)).clone();
        creation.egui_ctx.set_style_of(
            egui::Theme::Dark,
            match requested {
                WorkspaceKind::Photo => photo_style.clone(),
                WorkspaceKind::Canvas => canvas_style.clone(),
            },
        );
        let app = Self {
            photo,
            canvas,
            active: requested,
            photo_style,
            canvas_style,
            open_document_receiver,
            photo_document_sender,
            canvas_document_sender,
            trace: perf::FrameTrace::from_environment(),
        };
        #[cfg(target_os = "macos")]
        {
            let mut app = app;
            app.show_active_native_menu();
            app
        }
        #[cfg(not(target_os = "macos"))]
        {
            app
        }
    }

    #[cfg(target_os = "macos")]
    fn show_active_native_menu(&mut self) {
        match self.active {
            WorkspaceKind::Photo => self.photo.show_spectrum_menu(),
            WorkspaceKind::Canvas => self.canvas.show_spectrum_menu(),
        }
    }

    fn switch_to(&mut self, next: WorkspaceKind, context: &egui::Context) {
        if self.active == next {
            return;
        }
        match self.active {
            WorkspaceKind::Photo => self.photo.suspend_for_spectrum(),
            WorkspaceKind::Canvas => self.canvas.suspend_for_spectrum(),
        }
        self.active = next;
        context.set_style_of(
            egui::Theme::Dark,
            match next {
                WorkspaceKind::Photo => self.photo_style.clone(),
                WorkspaceKind::Canvas => self.canvas_style.clone(),
            },
        );
        #[cfg(target_os = "macos")]
        self.show_active_native_menu();
        context.request_repaint();
    }

    fn receive_open_documents(&mut self, context: &egui::Context) {
        let paths: Vec<_> = self.open_document_receiver.try_iter().collect();
        for path in paths {
            let Some(kind) = WorkspaceKind::from_path(&path) else {
                continue;
            };
            let sent = match kind {
                WorkspaceKind::Photo => self.photo_document_sender.send(path),
                WorkspaceKind::Canvas => self.canvas_document_sender.send(path),
            };
            if sent.is_ok() {
                self.switch_to(kind, context);
            }
        }
    }
}

impl eframe::App for SpectrumApp {
    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        let mut trace_sample = self.trace.as_mut().map(|trace| trace.start(root.ctx()));
        self.receive_open_documents(root.ctx());
        if let Some(sample) = &mut trace_sample {
            sample.documents_done();
        }
        let mut next = self.active;
        egui::Panel::top("spectrum-workspace-switcher").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Spectrum");
                ui.separator();
                if ui
                    .selectable_label(self.active == WorkspaceKind::Photo, "Photos")
                    .clicked()
                {
                    next = WorkspaceKind::Photo;
                }
                if ui
                    .selectable_label(self.active == WorkspaceKind::Canvas, "Canvas")
                    .clicked()
                {
                    next = WorkspaceKind::Canvas;
                }
            });
        });
        self.switch_to(next, root.ctx());
        if let Some(sample) = &mut trace_sample {
            sample.switcher_done();
        }
        match self.active {
            WorkspaceKind::Photo => {
                self.canvas.poll_background_for_spectrum(root.ctx());
                if let Some(sample) = &mut trace_sample {
                    sample.inactive_done();
                }
                self.photo.ui(root, frame);
            }
            WorkspaceKind::Canvas => {
                self.photo.poll_background_for_spectrum(root.ctx());
                if let Some(sample) = &mut trace_sample {
                    sample.inactive_done();
                }
                self.canvas.ui(root, frame);
            }
        }
        if let (Some(trace), Some(sample)) = (&mut self.trace, trace_sample) {
            trace.finish(sample, self.active);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(ACTIVE_WORKSPACE_KEY, self.active.storage_value().into());
        self.photo.save(storage);
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.photo.on_exit(gl);
        self.canvas.on_exit(gl);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() -> eframe::Result {
    let startup_path = std::env::args_os().nth(1).map(PathBuf::from);
    let (_, open_document_receiver) = mpsc::channel();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 940.0])
            .with_min_inner_size([1060.0, 680.0])
            .with_icon(spectrum_icon()),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Spectrum",
        options,
        Box::new(move |creation| {
            Ok(Box::new(SpectrumApp::new(
                creation,
                startup_path.clone(),
                open_document_receiver,
            )))
        }),
    )
}

#[cfg(target_os = "macos")]
fn main() -> eframe::Result {
    macos::run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_existing_project_formats_to_their_workspaces() {
        assert_eq!(
            WorkspaceKind::from_path(std::path::Path::new("library.lumen")),
            Some(WorkspaceKind::Photo)
        );
        assert_eq!(
            WorkspaceKind::from_path(std::path::Path::new("draft.prism")),
            Some(WorkspaceKind::Canvas)
        );
        assert_eq!(
            WorkspaceKind::from_path(std::path::Path::new("old.mica")),
            Some(WorkspaceKind::Canvas)
        );
    }
}
