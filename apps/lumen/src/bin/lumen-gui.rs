#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Color32, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};
use image::{DynamicImage, imageops::FilterType};
use lumen_core::{
    Adjustments, ColorGrade, Command, CropRect, CurvePoint, ExportFormat, Photo, PickState,
    Project, SpotRemoval, ToneCurve, Workspace,
    engine::{RenderOptions, decode_photo, render_image, render_photo},
    project::is_supported_image,
};

#[path = "lumen_gui/canvas.rs"]
mod canvas;
#[path = "lumen_gui/dialogs.rs"]
mod dialogs;
#[path = "lumen_gui/helpers.rs"]
mod helpers;
#[path = "lumen_gui/history.rs"]
mod history;
#[path = "lumen_gui/inspector.rs"]
mod inspector;
#[path = "lumen_gui/library.rs"]
mod library;
#[cfg(target_os = "macos")]
#[path = "lumen_gui/macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
#[path = "lumen_gui/macos_menu_spec.rs"]
mod macos_menu_spec;
#[path = "lumen_gui/state.rs"]
mod state;
#[path = "lumen_gui/terminal.rs"]
mod terminal;
#[path = "lumen_gui/toolbar.rs"]
mod toolbar;
use helpers::*;
use terminal::TerminalDock;

const ACCENT: Color32 = Color32::from_rgb(174, 123, 255);
const ACCENT_COOL: Color32 = Color32::from_rgb(121, 161, 255);
const PANEL: Color32 = Color32::from_rgb(27, 29, 33);
const SURFACE: Color32 = Color32::from_rgb(34, 37, 42);
const SURFACE_RAISED: Color32 = Color32::from_rgb(43, 46, 52);
const CANVAS: Color32 = Color32::from_rgb(12, 13, 16);
const RECENT_CATALOGS_KEY: &str = "recent-catalogs-v1";
const HSL_NAMES: [&str; 8] = [
    "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta",
];
const HSL_COLORS: [Color32; 8] = [
    Color32::from_rgb(220, 73, 73),
    Color32::from_rgb(230, 132, 54),
    Color32::from_rgb(224, 196, 63),
    Color32::from_rgb(83, 181, 87),
    Color32::from_rgb(59, 183, 187),
    Color32::from_rgb(74, 119, 224),
    Color32::from_rgb(145, 83, 205),
    Color32::from_rgb(211, 73, 159),
];

#[derive(Clone, Copy)]
enum CropHandle {
    Move,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy)]
struct CropDrag {
    handle: CropHandle,
    start: CropRect,
    pointer: Pos2,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CompareMode {
    #[default]
    Edited,
    SideBySide,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FilmFilter {
    #[default]
    All,
    Keeps,
    Rejects,
}

#[derive(Clone)]
enum CatalogSwitch {
    Open(PathBuf),
}

#[derive(Clone)]
struct Histogram {
    red: [u32; 256],
    green: [u32; 256],
    blue: [u32; 256],
    luma: [u32; 256],
}

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([1060.0, 680.0])
            .with_icon(lumen_icon()),
        centered: true,
        ..Default::default()
    }
}

fn lumen_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../../assets/branding/lumen-app-icon.png"
    ))
    .expect("bundled Lumen icon must be a valid PNG")
}

#[cfg(not(target_os = "macos"))]
fn main() -> eframe::Result {
    let initial_catalog = std::env::args_os().nth(1).map(PathBuf::from);
    let (_, open_document_receiver) = std::sync::mpsc::channel();
    eframe::run_native(
        "Lumen",
        native_options(),
        Box::new(move |creation| {
            Ok(Box::new(LumenApp::new(
                creation,
                initial_catalog.clone(),
                open_document_receiver,
            )))
        }),
    )
}

#[cfg(target_os = "macos")]
fn main() -> eframe::Result {
    let initial_catalog = std::env::args_os().nth(1).map(PathBuf::from);
    macos::run(initial_catalog)
}

struct LumenApp {
    workspace: Workspace,
    preview: Option<TextureHandle>,
    preview_source: Option<(u64, DynamicImage)>,
    preview_fast_source: Option<(u64, DynamicImage)>,
    preview_id: Option<u64>,
    preview_fast: bool,
    preview_adjustments: Adjustments,
    preview_layout_size: Option<Vec2>,
    original_preview: Option<TextureHandle>,
    original_preview_id: Option<u64>,
    histogram: Option<Histogram>,
    thumbnails: HashMap<u64, TextureHandle>,
    draft: Adjustments,
    draft_id: Option<u64>,
    status: String,
    error: bool,
    zoom: f32,
    pan: Vec2,
    compare_mode: CompareMode,
    film_filter: FilmFilter,
    library_mode: bool,
    active_batch: Option<u64>,
    rename_batch: Option<(u64, String)>,
    hsl_band: usize,
    curve_channel: usize,
    grade_range: usize,
    reset_confirmation: bool,
    remove_confirmation: bool,
    pending_catalog_switch: Option<CatalogSwitch>,
    recent_catalogs: Vec<PathBuf>,
    selected_ids: BTreeSet<u64>,
    selection_anchor: Option<usize>,
    crop_mode: bool,
    crop_draft: CropRect,
    crop_drag: Option<CropDrag>,
    spot_mode: bool,
    spot_radius: f32,
    spot_stroke_start: Option<usize>,
    export_open: bool,
    export_format: ExportFormat,
    export_quality: u8,
    export_max_size: u32,
    export_directory: Option<PathBuf>,
    preset_name: String,
    collaboration_poll_at: Instant,
    history_open: bool,
    history_selected: Option<spectrum_revisions::RevisionId>,
    history_scroll_to_current: bool,
    open_document_receiver: Receiver<PathBuf>,
    terminal: TerminalDock,
    #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
    native_terminal: spectrum_terminal::native_ghostty::NativeTerminalHost,
    #[cfg(target_os = "macos")]
    native_menu: macos::NativeMenuBridge,
}

impl LumenApp {
    fn new(
        creation: &eframe::CreationContext<'_>,
        initial_catalog: Option<PathBuf>,
        open_document_receiver: Receiver<PathBuf>,
        #[cfg(target_os = "macos")] native_menu: macos::NativeMenuBridge,
    ) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = PANEL;
        visuals.window_fill = PANEL;
        visuals.selection.bg_fill = ACCENT;
        visuals.selection.stroke.color = Color32::BLACK;
        visuals.extreme_bg_color = CANVAS;
        visuals.faint_bg_color = SURFACE;
        visuals.widgets.noninteractive.bg_fill = SURFACE;
        visuals.widgets.inactive.bg_fill = SURFACE_RAISED;
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(61, 64, 71);
        visuals.widgets.active.bg_fill = Color32::from_rgb(61, 49, 82);
        visuals.widgets.inactive.corner_radius = 5.0.into();
        visuals.widgets.hovered.corner_radius = 5.0.into();
        visuals.widgets.active.corner_radius = 5.0.into();
        creation.egui_ctx.set_visuals(visuals);
        creation.egui_ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = Vec2::new(8.0, 7.0);
            style.spacing.button_padding = Vec2::new(10.0, 6.0);
        });
        #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
        let native_terminal =
            spectrum_terminal::native_ghostty::NativeTerminalHost::from_environment(
                creation,
                spectrum_terminal::native_ghostty::NativeTerminalConfig::new(
                    "LUMEN_EXPERIMENTAL_GHOSTTY",
                    "LUMEN_GHOSTTY_BRIDGE",
                ),
            );
        #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
        let native_terminal_ready = native_terminal.is_ready();
        let recent_catalogs = creation
            .storage
            .and_then(|storage| storage.get_string(RECENT_CATALOGS_KEY))
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default();
        let (workspace, startup_status, startup_error) = match initial_catalog.as_deref() {
            Some(path) => match state::open_local_workspace(path) {
                Ok(workspace) => (workspace, format!("Opened {}", path.display()), false),
                Err(error) => (
                    state::create_managed_workspace(Project::default()).unwrap_or_default(),
                    format!("Could not open project: {error:#}"),
                    true,
                ),
            },
            None => match state::create_managed_workspace(Project::default()) {
                Ok(workspace) => (workspace, "Created a new Lumen project".into(), false),
                Err(error) => (
                    Workspace::default(),
                    format!("Could not create local project: {error:#}"),
                    true,
                ),
            },
        };
        let mut app = Self {
            workspace,
            preview: None,
            preview_source: None,
            preview_fast_source: None,
            preview_id: None,
            preview_fast: false,
            preview_adjustments: Adjustments::default(),
            preview_layout_size: None,
            original_preview: None,
            original_preview_id: None,
            histogram: None,
            thumbnails: HashMap::new(),
            draft: Adjustments::default(),
            draft_id: None,
            status: startup_status,
            error: startup_error,
            zoom: 1.0,
            pan: Vec2::ZERO,
            compare_mode: CompareMode::Edited,
            film_filter: FilmFilter::All,
            library_mode: true,
            active_batch: None,
            rename_batch: None,
            hsl_band: 0,
            curve_channel: 0,
            grade_range: 1,
            reset_confirmation: false,
            remove_confirmation: false,
            pending_catalog_switch: None,
            recent_catalogs,
            selected_ids: BTreeSet::new(),
            selection_anchor: None,
            crop_mode: false,
            crop_draft: CropRect::default(),
            crop_drag: None,
            spot_mode: false,
            spot_radius: 0.025,
            spot_stroke_start: None,
            export_open: false,
            export_format: ExportFormat::Jpeg,
            export_quality: 92,
            export_max_size: 0,
            export_directory: None,
            preset_name: String::new(),
            collaboration_poll_at: Instant::now(),
            history_open: false,
            history_selected: None,
            history_scroll_to_current: false,
            open_document_receiver,
            terminal: TerminalDock::new(
                #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
                native_terminal_ready,
                #[cfg(not(all(target_os = "macos", feature = "ghostty-terminal")))]
                false,
            ),
            #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
            native_terminal,
            #[cfg(target_os = "macos")]
            native_menu,
        };
        #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
        if let Some(diagnostic) = app.native_terminal.fallback_diagnostic() {
            app.status = diagnostic.to_owned();
            app.error = true;
        }
        if let Some(path) = app.workspace.catalog_path.clone() {
            app.remember_catalog(path);
        }
        app
    }
}

impl eframe::App for LumenApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        self.receive_open_documents(&context);
        self.sync_agent_collaborations(&context);
        self.sync_draft();
        self.poll_terminal(&context);
        #[cfg(target_os = "macos")]
        self.process_native_menu_actions(&context);
        self.handle_drop_and_shortcuts(&context);
        spectrum_history_ui::reserve_history_shortcut();
        self.toolbar(ui);
        self.status_bar(ui);
        if self.history_open {
            #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
            self.native_terminal.hide_all();
            self.history_view(ui);
        } else if self.terminal.visible() {
            self.terminal_panel(ui);
        } else if self.library_mode {
            #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
            self.native_terminal.hide_all();
            self.library_canvas(ui);
        } else {
            #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
            self.native_terminal.hide_all();
            self.filmstrip(ui);
            self.inspector(ui);
            self.canvas(ui);
        }
        self.confirmation_window(&context);
        self.remove_confirmation_window(&context);
        self.catalog_switch_confirmation_window(&context);
        self.rename_batch_window(&context);
        self.export_window(&context);
        #[cfg(target_os = "macos")]
        self.sync_native_menu_state(&context);
        if !context.input(|input| input.raw.hovered_files.is_empty()) {
            let painter = context.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop-overlay"),
            ));
            let rect = context.content_rect().shrink(24.0);
            painter.rect_filled(rect, 12.0, Color32::from_black_alpha(210));
            painter.rect_stroke(
                rect,
                12.0,
                Stroke::new(3.0, ACCENT),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop to import photos",
                egui::FontId::proportional(28.0),
                Color32::WHITE,
            );
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(value) = serde_json::to_string(&self.recent_catalogs) {
            storage.set_string(RECENT_CATALOGS_KEY, value);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.workspace.checkpoint();
        self.terminal.shutdown();
        #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
        self.native_terminal.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_lumen_icon_is_the_full_resolution_capital_l_artwork() {
        let icon = lumen_icon();
        assert_eq!([icon.width, icon.height], [1_024, 1_024]);
        assert_eq!(icon.rgba.len(), 1_024 * 1_024 * 4);
        assert_eq!(icon.rgba[3], 0, "the app icon must have a masked corner");
        assert_eq!(
            icon.rgba[(512 * 1_024 + 512) * 4 + 3],
            255,
            "the app icon center must remain opaque"
        );
    }

    #[test]
    fn fast_preview_keeps_full_preview_fit_geometry() {
        let layout = Some(Vec2::new(1_260.0, 720.0));
        let available = Vec2::new(1_100.0, 700.0);
        let full = preview_fit_size(layout, Vec2::new(1_260.0, 720.0), available);
        let fast = preview_fit_size(layout, Vec2::new(672.0, 384.0), available);

        assert_eq!(fast, full);
    }

    #[test]
    fn shoot_dates_prefer_capture_range_and_fall_back_to_import() {
        assert_eq!(
            shoot_date_label(Some("2026-07-14"), Some("2026-07-16"), "2026-07-20"),
            "Shot July 14, 2026 – July 16, 2026"
        );
        assert_eq!(
            shoot_date_label(None, None, "2026-07-20"),
            "Added July 20, 2026"
        );
    }

    #[test]
    fn geometry_changes_are_not_treated_as_pixel_only_edits() {
        let original = Adjustments::default();
        let mut exposure = original.clone();
        exposure.exposure = 1.0;
        assert!(same_preview_geometry(&original, &exposure));

        let mut cropped = original.clone();
        cropped.crop = Some(CropRect {
            x: 0.1,
            y: 0.1,
            width: 0.8,
            height: 0.7,
        });
        assert!(!same_preview_geometry(&original, &cropped));
    }
}
