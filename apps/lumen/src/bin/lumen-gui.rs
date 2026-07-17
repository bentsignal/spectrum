#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use eframe::egui::{
    self, Color32, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};
use image::{DynamicImage, imageops::FilterType};
use lumen_core::{
    Adjustments, ColorGrade, Command, CropRect, CurvePoint, ExportFormat, Photo, PickState,
    SpotRemoval, ToneCurve, Workspace,
    engine::{RenderOptions, decode_photo, render_image, render_photo},
    project::is_supported_image,
};

#[path = "lumen_gui/canvas.rs"]
mod canvas;
#[path = "lumen_gui/dialogs.rs"]
mod dialogs;
#[path = "lumen_gui/helpers.rs"]
mod helpers;
#[path = "lumen_gui/inspector.rs"]
mod inspector;
#[path = "lumen_gui/library.rs"]
mod library;
#[path = "lumen_gui/state.rs"]
mod state;
#[path = "lumen_gui/toolbar.rs"]
mod toolbar;
use helpers::*;

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
    New(PathBuf),
    Open(PathBuf),
}

#[derive(Clone)]
struct Histogram {
    red: [u32; 256],
    green: [u32; 256],
    blue: [u32; 256],
    luma: [u32; 256],
}

fn main() -> eframe::Result {
    let initial_catalog = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([1060.0, 680.0]),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Lumen",
        options,
        Box::new(move |creation| Ok(Box::new(LumenApp::new(creation, initial_catalog.clone())))),
    )
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
}

impl LumenApp {
    fn new(creation: &eframe::CreationContext<'_>, initial_catalog: Option<PathBuf>) -> Self {
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
        let recent_catalogs = creation
            .storage
            .and_then(|storage| storage.get_string(RECENT_CATALOGS_KEY))
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default();
        let mut app = Self {
            workspace: Workspace::default(),
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
            status: "Drop photos anywhere, or choose Import Photos".into(),
            error: false,
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
        };
        if let Some(path) = initial_catalog
            && app.execute(Command::Open { path: path.clone() })
        {
            app.remember_catalog(path);
            app.reset_catalog_view(true);
        }
        app
    }
}

impl eframe::App for LumenApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        self.sync_draft();
        self.handle_drop_and_shortcuts(&context);
        self.toolbar(ui);
        self.status_bar(ui);
        if self.library_mode {
            self.library_canvas(ui);
        } else {
            self.filmstrip(ui);
            self.inspector(ui);
            self.canvas(ui);
        }
        self.confirmation_window(&context);
        self.remove_confirmation_window(&context);
        self.catalog_switch_confirmation_window(&context);
        self.rename_batch_window(&context);
        self.export_window(&context);
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
