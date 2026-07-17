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

    fn execute(&mut self, command: Command) -> bool {
        match self.workspace.execute(command) {
            Ok(output) => {
                self.status = output.message;
                self.error = false;
                true
            }
            Err(error) => {
                self.status = format!("{error:#}");
                self.error = true;
                false
            }
        }
    }

    fn execute_and_autosave(&mut self, command: Command) -> bool {
        let succeeded = self.execute(command);
        if succeeded
            && let Some(path) = self.workspace.catalog_path.clone()
            && let Err(error) = self.workspace.project.save(&path)
        {
            self.status = format!("change applied, but autosave failed: {error:#}");
            self.error = true;
        }
        succeeded
    }

    fn invalidate_selected(&mut self) {
        self.preview = None;
        self.preview_source = None;
        self.preview_fast_source = None;
        self.preview_id = None;
        self.preview_layout_size = None;
        self.original_preview = None;
        self.original_preview_id = None;
        self.histogram = None;
        if let Some(id) = self.workspace.project.selected {
            self.thumbnails.remove(&id);
        }
    }

    fn sync_draft(&mut self) {
        let selected = self.workspace.project.selected;
        if self.draft_id != selected {
            self.draft_id = selected;
            self.draft = self
                .workspace
                .project
                .selected_photo()
                .map(|photo| photo.adjustments.clone())
                .unwrap_or_default();
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
            self.crop_mode = false;
            self.crop_drag = None;
            self.spot_mode = false;
            self.spot_stroke_start = None;
            if let Some(id) = selected
                && self.selected_ids.is_empty()
            {
                self.selected_ids.insert(id);
            }
            self.invalidate_selected();
        }
    }

    fn finish_edit(&mut self, id: u64) {
        if self.execute_and_autosave(Command::SetAdjustments {
            id,
            adjustments: self.draft.clone(),
        }) {
            self.thumbnails.remove(&id);
        }
    }

    fn select(&mut self, id: u64) {
        self.selected_ids.clear();
        self.selected_ids.insert(id);
        if self.execute(Command::Select { id }) {
            self.draft_id = None;
            self.sync_draft();
        }
    }

    fn select_in_filmstrip(&mut self, id: u64, index: usize, modifiers: egui::Modifiers) {
        if modifiers.shift {
            let last = self.workspace.project.photos.len().saturating_sub(1);
            let anchor = self.selection_anchor.unwrap_or(index).min(last);
            let (start, end) = if anchor <= index {
                (anchor, index)
            } else {
                (index, anchor)
            };
            if !modifiers.command {
                self.selected_ids.clear();
            }
            for photo in &self.workspace.project.photos[start..=end] {
                self.selected_ids.insert(photo.id);
            }
        } else if modifiers.command {
            if !self.selected_ids.remove(&id) {
                self.selected_ids.insert(id);
            }
            if self.selected_ids.is_empty() {
                self.selected_ids.insert(id);
            }
            self.selection_anchor = Some(index);
        } else {
            self.selected_ids.clear();
            self.selected_ids.insert(id);
            self.selection_anchor = Some(index);
        }
        let active = if self.selected_ids.contains(&id) {
            Some(id)
        } else {
            self.selected_ids.iter().next().copied()
        };
        if let Some(active) = active
            && self.execute(Command::Select { id: active })
        {
            self.draft_id = None;
            self.sync_draft();
        }
    }

    fn selected_photo_ids(&self) -> Vec<u64> {
        if self.selected_ids.is_empty() {
            self.workspace.project.selected.into_iter().collect()
        } else {
            self.selected_ids.iter().copied().collect()
        }
    }

    fn set_pick(&mut self, ids: Vec<u64>, state: PickState) {
        if !ids.is_empty() {
            self.execute_and_autosave(Command::SetPick { ids, state });
        }
    }

    fn visible_photo_ids(&self) -> Vec<u64> {
        self.workspace
            .project
            .photos
            .iter()
            .filter(|photo| {
                self.active_batch
                    .is_none_or(|batch_id| photo.batch_id == Some(batch_id))
            })
            .filter(|photo| match self.film_filter {
                FilmFilter::All => true,
                FilmFilter::Keeps => photo.pick == PickState::Keep,
                FilmFilter::Rejects => photo.pick == PickState::Reject,
            })
            .map(|photo| photo.id)
            .collect()
    }

    fn import(&mut self, paths: Vec<PathBuf>) {
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|path| is_supported_image(path))
            .collect();
        if paths.is_empty() {
            self.status = "No supported images were selected".into();
            self.error = true;
            return;
        }
        let batch_count = self.workspace.project.batches.len();
        self.status = "Reading photo metadata and importing...".into();
        if self.execute_and_autosave(Command::Import { paths }) {
            self.thumbnails.clear();
            self.selected_ids.clear();
            self.selection_anchor = None;
            self.draft_id = None;
            self.sync_draft();
            let imported_batch = if self.workspace.project.batches.len() > batch_count {
                self.workspace.project.batches.last().map(|batch| batch.id)
            } else {
                None
            };
            self.active_batch = imported_batch.or_else(|| {
                self.workspace
                    .project
                    .selected_photo()
                    .and_then(|photo| photo.batch_id)
            });
            if let Some(batch_id) = imported_batch
                && let Some(photo_id) = self
                    .workspace
                    .project
                    .photos
                    .iter()
                    .find(|photo| photo.batch_id == Some(batch_id))
                    .map(|photo| photo.id)
            {
                self.select(photo_id);
            }
            self.library_mode = false;
        }
    }

    fn import_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter(
                "Photos",
                &["jpg", "jpeg", "png", "tif", "tiff", "webp", "arw"],
            )
            .pick_files()
        {
            self.import(paths);
        }
    }

    fn reset_catalog_view(&mut self, library_mode: bool) {
        self.thumbnails.clear();
        self.selected_ids.clear();
        self.selection_anchor = None;
        self.draft_id = None;
        self.preview = None;
        self.preview_source = None;
        self.preview_fast_source = None;
        self.original_preview = None;
        self.histogram = None;
        self.film_filter = FilmFilter::All;
        self.active_batch = None;
        self.library_mode = library_mode;
        self.sync_draft();
    }

    fn remember_catalog(&mut self, path: PathBuf) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        self.recent_catalogs.retain(|recent| recent != &path);
        self.recent_catalogs.insert(0, path);
        self.recent_catalogs.truncate(8);
    }

    fn request_catalog_switch(&mut self, action: CatalogSwitch) {
        if self.workspace.catalog_path.is_none() && !self.workspace.project.photos.is_empty() {
            self.pending_catalog_switch = Some(action);
        } else {
            self.apply_catalog_switch(action);
        }
    }

    fn apply_catalog_switch(&mut self, action: CatalogSwitch) {
        match action {
            CatalogSwitch::New(path) => {
                let name = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Untitled catalog")
                    .to_owned();
                if self.execute(Command::New { name })
                    && self.execute(Command::Save {
                        path: Some(path.clone()),
                    })
                {
                    self.remember_catalog(path);
                    self.reset_catalog_view(true);
                }
            }
            CatalogSwitch::Open(path) => {
                if self.execute(Command::Open { path: path.clone() }) {
                    self.remember_catalog(path);
                    self.reset_catalog_view(true);
                }
            }
        }
    }

    fn new_catalog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Lumen catalog", &["lumencatalog"])
            .set_file_name("New shoot.lumencatalog")
            .save_file()
        {
            self.request_catalog_switch(CatalogSwitch::New(path));
        }
    }

    fn open_catalog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Lumen catalog", &["lumencatalog"])
            .pick_file()
        {
            self.request_catalog_switch(CatalogSwitch::Open(path));
        }
    }

    fn save_catalog(&mut self, save_as: bool) {
        let needs_picker = save_as || self.workspace.catalog_path.is_none();
        let path = if needs_picker {
            rfd::FileDialog::new()
                .add_filter("Lumen catalog", &["lumencatalog"])
                .set_file_name(format!("{}.lumencatalog", self.workspace.project.name))
                .save_file()
        } else {
            None
        };
        if needs_picker && path.is_none() {
            return;
        }
        if self.execute(Command::Save { path })
            && let Some(path) = self.workspace.catalog_path.clone()
        {
            self.remember_catalog(path);
        }
    }

    fn open_export(&mut self) {
        if !self.selected_photo_ids().is_empty() {
            self.export_open = true;
        }
    }

    fn begin_crop(&mut self) {
        self.crop_draft = self.draft.crop.unwrap_or_default();
        self.crop_mode = true;
        self.spot_mode = false;
        self.compare_mode = CompareMode::Edited;
        self.crop_drag = None;
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.preview = None;
    }

    fn cancel_crop(&mut self) {
        self.crop_mode = false;
        self.crop_drag = None;
        self.preview = None;
    }

    fn apply_crop(&mut self) {
        let Some(id) = self.workspace.project.selected else {
            return;
        };
        self.draft.crop = Some(self.crop_draft.sanitized());
        self.crop_mode = false;
        self.crop_drag = None;
        self.preview = None;
        self.finish_edit(id);
    }

    fn preview_adjustments(&self) -> Adjustments {
        let mut adjustments = self.draft.clone();
        if self.crop_mode {
            adjustments.crop = None;
        }
        adjustments
    }

    fn ensure_preview(&mut self, context: &egui::Context) {
        let Some(id) = self.workspace.project.selected else {
            self.preview = None;
            return;
        };
        let preview_adjustments = self.preview_adjustments();
        let interacting = context.input(|input| input.pointer.primary_down());
        if self.preview.is_some()
            && self.preview_id == Some(id)
            && self.preview_adjustments == preview_adjustments
            && !(self.preview_fast && !interacting)
        {
            return;
        }
        let Some(photo) = self.workspace.project.selected_photo().cloned() else {
            return;
        };
        if self
            .preview_source
            .as_ref()
            .map(|(source_id, _)| *source_id)
            != Some(id)
        {
            match decode_photo(&photo, Some(1800)) {
                Ok(source) => {
                    let fast = if source.width() > 960 || source.height() > 960 {
                        source.resize(960, 960, FilterType::Triangle)
                    } else {
                        source.clone()
                    };
                    self.preview_fast_source = Some((id, fast));
                    self.preview_source = Some((id, source));
                }
                Err(error) => {
                    self.status = format!("preview failed: {error:#}");
                    self.error = true;
                    return;
                }
            }
        }
        let geometry_changed = self.preview_id == Some(id)
            && !same_preview_geometry(&self.preview_adjustments, &preview_adjustments);
        let use_fast = interacting && self.preview_id == Some(id) && !geometry_changed;
        let source = if use_fast {
            self.preview_fast_source.as_ref()
        } else {
            self.preview_source.as_ref()
        };
        if let Some((_, source)) = source {
            if self.original_preview_id != Some(id) {
                self.original_preview = Some(load_texture(
                    context,
                    format!("original-{id}"),
                    source.clone(),
                ));
                self.original_preview_id = Some(id);
            }
            let rendered = render_image(
                source.clone(),
                preview_adjustments.clone(),
                RenderOptions::default(),
            );
            self.histogram = Some(Histogram::from_image(&rendered));
            if !use_fast || self.preview_layout_size.is_none() {
                self.preview_layout_size =
                    Some(Vec2::new(rendered.width() as f32, rendered.height() as f32));
            }
            self.preview = Some(load_texture(context, format!("preview-{id}"), rendered));
            self.preview_id = Some(id);
            self.preview_fast = use_fast;
            self.preview_adjustments = preview_adjustments;
        }
    }

    fn ensure_thumbnail(&mut self, context: &egui::Context, id: u64) {
        if self.thumbnails.contains_key(&id) {
            return;
        }
        let Ok(photo) = self.workspace.project.photo(id) else {
            return;
        };
        if let Ok(rendered) = render_photo(
            photo,
            RenderOptions {
                max_size: Some(240),
            },
        ) {
            self.thumbnails.insert(
                id,
                load_texture(context, format!("thumbnail-{id}"), rendered),
            );
        }
    }

    fn handle_drop_and_shortcuts(&mut self, context: &egui::Context) {
        let dropped = context.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        if !dropped.is_empty() {
            self.import(dropped);
        }
        if context.input(|input| input.modifiers.command && input.key_pressed(egui::Key::S)) {
            self.save_catalog(context.input(|input| input.modifiers.shift));
        }
        if context.input(|input| input.modifiers.command && input.key_pressed(egui::Key::A))
            && !context.egui_wants_keyboard_input()
        {
            self.selected_ids = self.visible_photo_ids().into_iter().collect();
            self.selection_anchor = self.workspace.project.selected.and_then(|id| {
                self.workspace
                    .project
                    .photos
                    .iter()
                    .position(|photo| photo.id == id)
            });
        }
        let history = context.input(|input| {
            if input.modifiers.command && input.key_pressed(egui::Key::Z) {
                Some(input.modifiers.shift)
            } else {
                None
            }
        });
        if let Some(forward) = history
            && let Some(id) = self.workspace.project.selected
        {
            let command = if forward {
                Command::HistoryForward { id }
            } else {
                Command::HistoryBack { id }
            };
            if self.execute_and_autosave(command) {
                self.draft_id = None;
                self.sync_draft();
            }
        }
        let direction = context.input(|input| {
            if input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::ArrowUp) {
                -1
            } else if input.key_pressed(egui::Key::ArrowRight)
                || input.key_pressed(egui::Key::ArrowDown)
            {
                1
            } else {
                0
            }
        });
        if direction != 0 && !context.egui_wants_keyboard_input() {
            self.select_relative(direction);
        }
        if !context.egui_wants_keyboard_input() {
            let pick = context.input(|input| {
                if input.key_pressed(egui::Key::P) {
                    Some(PickState::Keep)
                } else if input.key_pressed(egui::Key::X) {
                    Some(PickState::Reject)
                } else if input.key_pressed(egui::Key::U) {
                    Some(PickState::Unmarked)
                } else {
                    None
                }
            });
            if let Some(state) = pick {
                self.set_pick(self.selected_photo_ids(), state);
            }
            if context.input(|input| input.key_pressed(egui::Key::C)) {
                self.compare_mode = if self.compare_mode == CompareMode::Edited {
                    CompareMode::SideBySide
                } else {
                    CompareMode::Edited
                };
            }
            if context.input(|input| input.key_pressed(egui::Key::Delete))
                && self.workspace.project.selected.is_some()
            {
                self.remove_confirmation = true;
            }
        }
    }

    fn select_relative(&mut self, direction: i32) {
        let visible = self.visible_photo_ids();
        if visible.is_empty() {
            return;
        }
        let current = self
            .workspace
            .project
            .selected
            .and_then(|id| visible.iter().position(|visible| *visible == id))
            .unwrap_or(0) as i32;
        let index = (current + direction).clamp(0, visible.len() as i32 - 1) as usize;
        self.select(visible[index]);
    }

    fn toolbar(&mut self, root: &mut egui::Ui) {
        let recent_catalogs = self.recent_catalogs.clone();
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, Color32::from_gray(49)))
                    .inner_margin(egui::Margin::symmetric(14, 9)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("LUMEN").strong().size(17.0).color(ACCENT));
                    ui.separator();
                    if ui.button("Import Photos").clicked() {
                        self.import_dialog();
                    }
                    ui.menu_button("Catalog", |ui| {
                        if ui.button("New Catalog...").clicked() {
                            ui.close();
                            self.new_catalog();
                        }
                        if ui.button("Open Catalog...").clicked() {
                            ui.close();
                            self.open_catalog();
                        }
                        if ui.button("Save").clicked() {
                            ui.close();
                            self.save_catalog(false);
                        }
                        if ui.button("Save As...").clicked() {
                            ui.close();
                            self.save_catalog(true);
                        }
                        ui.separator();
                        ui.label(
                            RichText::new("RECENT CATALOGS")
                                .size(10.0)
                                .color(Color32::GRAY),
                        );
                        if recent_catalogs.is_empty() {
                            ui.label(RichText::new("None yet").color(Color32::GRAY));
                        } else {
                            for path in &recent_catalogs {
                                if ui
                                    .add_enabled(
                                        path.exists(),
                                        egui::Button::new(catalog_label(path)),
                                    )
                                    .on_hover_text(path.display().to_string())
                                    .clicked()
                                {
                                    self.request_catalog_switch(CatalogSwitch::Open(path.clone()));
                                    ui.close();
                                }
                            }
                        }
                    });
                    if self.library_mode {
                        ui.separator();
                        ui.label(RichText::new("LIBRARY").strong().color(ACCENT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(current_catalog_name(&self.workspace))
                                    .color(Color32::GRAY),
                            );
                        });
                        return;
                    }
                    if ui.button("Library").clicked() {
                        self.library_mode = true;
                    }
                    ui.separator();
                    let (can_back, can_forward) = self
                        .workspace
                        .project
                        .selected_photo()
                        .map(|photo| (photo.can_history_back(), photo.can_history_forward()))
                        .unwrap_or_default();
                    if ui
                        .add_enabled(can_back, egui::Button::new("Back"))
                        .on_hover_text("Go back one edit (Cmd/Ctrl+Z)")
                        .clicked()
                        && let Some(id) = self.workspace.project.selected
                        && self.execute_and_autosave(Command::HistoryBack { id })
                    {
                        self.draft_id = None;
                        self.sync_draft();
                    }
                    if ui
                        .add_enabled(can_forward, egui::Button::new("Forward"))
                        .on_hover_text("Go forward one edit (Cmd/Ctrl+Shift+Z)")
                        .clicked()
                        && let Some(id) = self.workspace.project.selected
                        && self.execute_and_autosave(Command::HistoryForward { id })
                    {
                        self.draft_id = None;
                        self.sync_draft();
                    }
                    ui.separator();
                    if ui.button("Fit").clicked() {
                        self.zoom = 1.0;
                        self.pan = Vec2::ZERO;
                    }
                    if ui.button("-").clicked() {
                        self.zoom = (self.zoom / 1.25).max(0.25);
                    }
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    if ui.button("+").clicked() {
                        self.zoom = (self.zoom * 1.25).min(8.0);
                    }
                    ui.separator();
                    ui.selectable_value(&mut self.compare_mode, CompareMode::Edited, "Edited");
                    ui.selectable_value(
                        &mut self.compare_mode,
                        CompareMode::SideBySide,
                        "Before | After",
                    )
                    .on_hover_text("Compare original and edited photo (C)");
                    ui.separator();
                    let pick = self
                        .workspace
                        .project
                        .selected_photo()
                        .map(|photo| photo.pick)
                        .unwrap_or_default();
                    if ui
                        .selectable_label(pick == PickState::Keep, "+ Keep")
                        .on_hover_text("Mark selected photos as keeps (P)")
                        .clicked()
                    {
                        self.set_pick(self.selected_photo_ids(), PickState::Keep);
                    }
                    if ui
                        .selectable_label(pick == PickState::Reject, "x Reject")
                        .on_hover_text("Mark selected photos as rejects (X)")
                        .clicked()
                    {
                        self.set_pick(self.selected_photo_ids(), PickState::Reject);
                    }
                    if ui
                        .add_enabled(
                            self.workspace.project.selected.is_some(),
                            egui::Button::new("Remove"),
                        )
                        .on_hover_text("Remove selected photos from this catalog")
                        .clicked()
                    {
                        self.remove_confirmation = true;
                    }
                    ui.separator();
                    if self.crop_mode {
                        if ui
                            .button(RichText::new("Apply Crop").color(ACCENT))
                            .clicked()
                        {
                            self.apply_crop();
                        }
                        if ui.button("Cancel").clicked() {
                            self.cancel_crop();
                        }
                    } else if ui
                        .add_enabled(
                            self.workspace.project.selected.is_some(),
                            egui::Button::new("Crop"),
                        )
                        .clicked()
                    {
                        self.begin_crop();
                    }
                    if ui
                        .add_enabled(
                            self.workspace.project.selected.is_some(),
                            egui::Button::new(format!(
                                "Export{}",
                                match self.selected_photo_ids().len() {
                                    0 | 1 => String::new(),
                                    count => format!(" {count}"),
                                }
                            )),
                        )
                        .clicked()
                    {
                        self.open_export();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(current_catalog_name(&self.workspace))
                                .color(Color32::GRAY),
                        );
                    });
                });
            });
    }

    fn filmstrip(&mut self, root: &mut egui::Ui) {
        let context = root.ctx().clone();
        egui::Panel::left("filmstrip")
            .resizable(true)
            .default_size(172.0)
            .size_range(138.0..=240.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(10, 10)),
            )
            .show(root, |ui| {
                let batch_photo_count = self
                    .workspace
                    .project
                    .photos
                    .iter()
                    .filter(|photo| {
                        self.active_batch
                            .is_none_or(|batch_id| photo.batch_id == Some(batch_id))
                    })
                    .count();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("PHOTOS")
                            .strong()
                            .size(12.0)
                            .color(Color32::GRAY),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let selected = self.selected_photo_ids().len();
                        ui.label(if selected > 1 {
                            format!("{selected} selected")
                        } else {
                            batch_photo_count.to_string()
                        });
                    });
                });
                ui.label(
                    RichText::new("Shift for range | Cmd/Ctrl to toggle")
                        .size(9.0)
                        .color(Color32::from_gray(115)),
                );
                if let Some(batch) = self
                    .active_batch
                    .and_then(|id| self.workspace.project.batch(id).ok())
                {
                    ui.label(RichText::new(&batch.name).size(11.0).strong().color(ACCENT));
                }
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.film_filter, FilmFilter::All, "All");
                    ui.selectable_value(&mut self.film_filter, FilmFilter::Keeps, "Keeps");
                    ui.selectable_value(&mut self.film_filter, FilmFilter::Rejects, "Rejects");
                });
                ui.separator();
                let mut photos: Vec<_> = self
                    .workspace
                    .project
                    .photos
                    .iter()
                    .enumerate()
                    .filter(|(_, photo)| {
                        self.active_batch
                            .is_none_or(|batch_id| photo.batch_id == Some(batch_id))
                    })
                    .filter(|(_, photo)| match self.film_filter {
                        FilmFilter::All => true,
                        FilmFilter::Keeps => photo.pick == PickState::Keep,
                        FilmFilter::Rejects => photo.pick == PickState::Reject,
                    })
                    .map(|(index, photo)| (index, photo.id, photo.name.clone(), photo.pick))
                    .collect();
                if self.film_filter == FilmFilter::All {
                    photos.sort_by_key(|(index, _, _, pick)| {
                        (if *pick == PickState::Keep { 0 } else { 1 }, *index)
                    });
                }
                if photos.is_empty() {
                    ui.label(RichText::new("No photos in this view").color(Color32::GRAY));
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (project_index, id, name, pick) in photos {
                            self.ensure_thumbnail(&context, id);
                            let selected = self.selected_ids.contains(&id);
                            let active = self.workspace.project.selected == Some(id);
                            let frame = egui::Frame::new()
                                .fill(if selected {
                                    Color32::from_rgb(50, 41, 67)
                                } else if pick == PickState::Keep {
                                    Color32::from_rgb(31, 48, 43)
                                } else if pick == PickState::Reject {
                                    Color32::from_rgb(45, 31, 34)
                                } else {
                                    SURFACE
                                })
                                .stroke(if active {
                                    Stroke::new(2.0, ACCENT)
                                } else if selected {
                                    Stroke::new(1.5, ACCENT_COOL)
                                } else {
                                    Stroke::new(1.0, Color32::from_gray(50))
                                })
                                .corner_radius(5.0)
                                .inner_margin(6);
                            let mut pick_action = None;
                            let inner = frame.show(ui, |ui| {
                                if let Some(texture) = self.thumbnails.get(&id) {
                                    let width = ui.available_width();
                                    ui.add(egui::Image::new(texture).fit_to_exact_size(fit_size(
                                        texture.size_vec2(),
                                        Vec2::new(width, 108.0),
                                    )));
                                } else {
                                    ui.allocate_space(Vec2::new(ui.available_width(), 84.0));
                                }
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(shorten(&name, 18)).size(11.0));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| match pick {
                                            PickState::Keep => {
                                                if ui
                                                    .selectable_label(true, "+")
                                                    .on_hover_text("Kept - click to unmark")
                                                    .clicked()
                                                {
                                                    pick_action = Some(PickState::Unmarked);
                                                }
                                            }
                                            PickState::Reject => {
                                                if ui
                                                    .selectable_label(true, "x")
                                                    .on_hover_text("Rejected - click to unmark")
                                                    .clicked()
                                                {
                                                    pick_action = Some(PickState::Unmarked);
                                                }
                                            }
                                            PickState::Unmarked => {}
                                        },
                                    );
                                });
                            });
                            if let Some(state) = pick_action {
                                self.set_pick(vec![id], state);
                            } else if inner.response.interact(Sense::click()).clicked() {
                                let modifiers = ui.input(|input| input.modifiers);
                                self.select_in_filmstrip(id, project_index, modifiers);
                            }
                            ui.add_space(5.0);
                        }
                    });
            });
    }

    fn open_batch(&mut self, batch_id: u64) {
        let first = self
            .workspace
            .project
            .photos
            .iter()
            .find(|photo| photo.batch_id == Some(batch_id))
            .map(|photo| photo.id);
        self.active_batch = Some(batch_id);
        self.library_mode = false;
        self.film_filter = FilmFilter::All;
        if let Some(id) = first {
            self.select(id);
        }
    }

    fn library_canvas(&mut self, root: &mut egui::Ui) {
        let context = root.ctx().clone();
        let mut batches: Vec<_> = self
            .workspace
            .project
            .batches
            .iter()
            .map(|batch| {
                let photos = self
                    .workspace
                    .project
                    .photos
                    .iter()
                    .filter(|photo| photo.batch_id == Some(batch.id))
                    .map(|photo| (photo.id, photo.name.clone(), photo.pick))
                    .collect::<Vec<_>>();
                (
                    batch.id,
                    batch.name.clone(),
                    batch.captured_date.clone(),
                    photos,
                )
            })
            .filter(|(_, _, _, photos)| !photos.is_empty())
            .collect();
        batches.sort_by_key(|(id, _, date, _)| {
            (date.clone().unwrap_or_else(|| "9999-99-99".into()), *id)
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(CANVAS).inner_margin(24))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(&self.workspace.project.name)
                                .size(24.0)
                                .color(Color32::from_gray(230)),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} shoots  |  {} photos",
                                batches.len(),
                                self.workspace.project.photos.len()
                            ))
                            .size(12.0)
                            .color(Color32::GRAY),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Import a Shoot").clicked() {
                            self.import_dialog();
                        }
                    });
                });
                ui.add_space(12.0);

                if batches.is_empty() {
                    let available = ui.available_rect_before_wrap();
                    ui.allocate_rect(available, Sense::hover());
                    let content = Rect::from_center_size(
                        available.center(),
                        Vec2::new(available.width().min(520.0), 150.0),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(content)
                            .layout(egui::Layout::top_down(egui::Align::Center)),
                        |ui| {
                            ui.label(
                                RichText::new("Your timeline starts here")
                                    .size(24.0)
                                    .color(Color32::from_gray(225)),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(
                                    "Import a shoot and Lumen will place it on the timeline.",
                                )
                                .size(13.0)
                                .color(Color32::GRAY),
                            );
                            ui.add_space(14.0);
                            if ui.button("Choose Photos").clicked() {
                                self.import_dialog();
                            }
                        },
                    );
                    return;
                }

                ui.label(
                    RichText::new("EARLIER   <   SHOOT TIMELINE   >   LATER")
                        .size(10.0)
                        .strong()
                        .color(ACCENT),
                );
                ui.add_space(8.0);

                let column_height = ui.available_height().max(280.0);
                let mut open_batch = None;
                let mut rename_batch = None;
                egui::ScrollArea::horizontal()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            for (batch_id, name, date, photos) in &batches {
                                for (photo_id, _, _) in photos {
                                    self.ensure_thumbnail(&context, *photo_id);
                                }
                                egui::Frame::new()
                                    .fill(SURFACE)
                                    .stroke(Stroke::new(1.0, Color32::from_gray(54)))
                                    .corner_radius(8.0)
                                    .inner_margin(10)
                                    .show(ui, |ui| {
                                        ui.vertical(|ui| {
                                            ui.set_width(190.0);
                                            ui.set_height(column_height - 24.0);
                                            if ui
                                                .add_sized(
                                                    [190.0, 28.0],
                                                    egui::Button::new(
                                                        RichText::new(name).strong().color(ACCENT),
                                                    ),
                                                )
                                                .on_hover_text("Open this shoot")
                                                .clicked()
                                            {
                                                open_batch = Some(*batch_id);
                                            }
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(
                                                        date.as_deref().unwrap_or("Date unknown"),
                                                    )
                                                    .size(10.0)
                                                    .color(Color32::GRAY),
                                                );
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        if ui.small_button("Rename").clicked() {
                                                            rename_batch =
                                                                Some((*batch_id, name.clone()));
                                                        }
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "{} photos",
                                                                photos.len()
                                                            ))
                                                            .size(10.0)
                                                            .color(Color32::GRAY),
                                                        );
                                                    },
                                                );
                                            });
                                            ui.separator();
                                            egui::ScrollArea::vertical()
                                                .id_salt(("batch-column", batch_id))
                                                .max_height((column_height - 100.0).max(160.0))
                                                .auto_shrink([false, false])
                                                .show(ui, |ui| {
                                                    for (photo_id, photo_name, pick) in photos {
                                                        if let Some(texture) =
                                                            self.thumbnails.get(photo_id)
                                                        {
                                                            let size = fit_size(
                                                                texture.size_vec2(),
                                                                Vec2::new(180.0, 112.0),
                                                            );
                                                            if ui
                                                                .add(
                                                                    egui::Image::new(texture)
                                                                        .fit_to_exact_size(size)
                                                                        .sense(Sense::click()),
                                                                )
                                                                .on_hover_text("Open this shoot")
                                                                .clicked()
                                                            {
                                                                open_batch = Some(*batch_id);
                                                            }
                                                        }
                                                        ui.horizontal(|ui| {
                                                            ui.label(
                                                                RichText::new(shorten(
                                                                    photo_name, 20,
                                                                ))
                                                                .size(10.0),
                                                            );
                                                            ui.with_layout(
                                                                egui::Layout::right_to_left(
                                                                    egui::Align::Center,
                                                                ),
                                                                |ui| match pick {
                                                                    PickState::Keep => {
                                                                        ui.label(
                                                                            RichText::new("+")
                                                                                .strong()
                                                                                .color(ACCENT),
                                                                        );
                                                                    }
                                                                    PickState::Reject => {
                                                                        ui.label(
                                                                            RichText::new("x")
                                                                                .color(
                                                                                Color32::from_rgb(
                                                                                    225, 112, 120,
                                                                                ),
                                                                            ),
                                                                        );
                                                                    }
                                                                    PickState::Unmarked => {}
                                                                },
                                                            );
                                                        });
                                                        ui.add_space(8.0);
                                                    }
                                                });
                                        });
                                    });
                                ui.add_space(8.0);
                            }
                        });
                    });
                if let Some(batch_id) = open_batch {
                    self.open_batch(batch_id);
                }
                if let Some(batch) = rename_batch {
                    self.rename_batch = Some(batch);
                }
            });
    }

    fn inspector(&mut self, root: &mut egui::Ui) {
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(330.0)
            .size_range(300.0..=430.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, Color32::from_gray(48)))
                    .inner_margin(egui::Margin::same(14)),
            )
            .show(root, |ui| {
                let Some(id) = self.workspace.project.selected else {
                    ui.heading("Develop");
                    ui.label(RichText::new("Select a photo to edit").color(Color32::GRAY));
                    return;
                };
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.histogram_ui(ui);
                        self.photo_details_ui(ui, id);
                        ui.add_space(4.0);
                        ui.heading("Develop");
                        ui.add_space(3.0);
                        let mut draft = self.draft.clone();
                        let mut changed = false;
                        let mut commit = false;
                        egui::CollapsingHeader::new("Light")
                            .default_open(true)
                            .show(ui, |ui| {
                                slider(
                                    ui,
                                    "Exposure",
                                    &mut draft.exposure,
                                    -5.0..=5.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Contrast",
                                    &mut draft.contrast,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Highlights",
                                    &mut draft.highlights,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Shadows",
                                    &mut draft.shadows,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Whites",
                                    &mut draft.whites,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Blacks",
                                    &mut draft.blacks,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                            });
                        egui::CollapsingHeader::new("Color")
                            .default_open(true)
                            .show(ui, |ui| {
                                slider(
                                    ui,
                                    "Temperature",
                                    &mut draft.temperature,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Tint",
                                    &mut draft.tint,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Vibrance",
                                    &mut draft.vibrance,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Saturation",
                                    &mut draft.saturation,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                            });
                        egui::CollapsingHeader::new("Color Grading")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for (index, label) in
                                        ["Shadows", "Midtones", "Highlights"].into_iter().enumerate()
                                    {
                                        if ui
                                            .selectable_label(self.grade_range == index, label)
                                            .clicked()
                                        {
                                            self.grade_range = index;
                                        }
                                    }
                                });
                                {
                                    let grade = match self.grade_range {
                                        0 => &mut draft.color_grading.shadows,
                                        1 => &mut draft.color_grading.midtones,
                                        _ => &mut draft.color_grading.highlights,
                                    };
                                    ui.horizontal(|ui| {
                                        grade_swatch(ui, *grade);
                                        ui.label(
                                            RichText::new("Tonal tint")
                                                .size(10.0)
                                                .color(Color32::GRAY),
                                        );
                                        if ui.small_button("Reset range").clicked() {
                                            *grade = ColorGrade::default();
                                            changed = true;
                                            commit = true;
                                        }
                                    });
                                    slider(
                                        ui,
                                        "Hue",
                                        &mut grade.hue,
                                        0.0..=360.0,
                                        &mut changed,
                                        &mut commit,
                                    );
                                    slider(
                                        ui,
                                        "Saturation",
                                        &mut grade.saturation,
                                        0.0..=100.0,
                                        &mut changed,
                                        &mut commit,
                                    );
                                    slider(
                                        ui,
                                        "Luminance",
                                        &mut grade.luminance,
                                        -100.0..=100.0,
                                        &mut changed,
                                        &mut commit,
                                    );
                                }
                                slider(
                                    ui,
                                    "Balance",
                                    &mut draft.color_grading.balance,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                            });
                        egui::CollapsingHeader::new("Presence & Detail")
                            .default_open(true)
                            .show(ui, |ui| {
                                slider(
                                    ui,
                                    "Texture",
                                    &mut draft.texture,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Clarity",
                                    &mut draft.clarity,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Dehaze",
                                    &mut draft.dehaze,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Sharpening",
                                    &mut draft.sharpening,
                                    0.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Noise Reduction",
                                    &mut draft.noise_reduction,
                                    0.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Vignette",
                                    &mut draft.vignette,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                            });
                        egui::CollapsingHeader::new("Crop & Transform")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if self.crop_mode {
                                        if ui
                                            .button(RichText::new("Apply Crop").color(ACCENT))
                                            .clicked()
                                        {
                                            self.apply_crop();
                                        }
                                        if ui.button("Cancel").clicked() {
                                            self.cancel_crop();
                                        }
                                    } else if ui.button("Edit Crop on Image").clicked() {
                                        self.begin_crop();
                                    }
                                    if draft.crop.is_some()
                                        && ui.small_button("Clear").clicked()
                                    {
                                        draft.crop = None;
                                        changed = true;
                                        commit = true;
                                    }
                                });
                                if self.crop_mode {
                                    ui.label(
                                        RichText::new(
                                            "Drag corners, edges, or the crop interior. The rule-of-thirds overlay updates live.",
                                        )
                                        .size(10.0)
                                        .color(Color32::GRAY),
                                    );
                                    let source_aspect = self
                                        .workspace
                                        .project
                                        .photo(id)
                                        .map(|photo| {
                                            let aspect = photo.width as f32 / photo.height.max(1) as f32;
                                            if matches!(draft.rotation, 90 | 270) { 1.0 / aspect } else { aspect }
                                        })
                                        .unwrap_or(1.0);
                                    ui.horizontal(|ui| {
                                        ui.label("Aspect");
                                        for (label, ratio) in
                                            [("Free", None), ("1:1", Some(1.0)), ("4:5", Some(0.8)), ("16:9", Some(16.0 / 9.0))]
                                        {
                                            if ui.small_button(label).clicked()
                                                && let Some(ratio) = ratio
                                            {
                                                set_crop_aspect(&mut self.crop_draft, ratio, source_aspect);
                                            }
                                        }
                                    });
                                } else {
                                    ui.label(
                                        RichText::new(if draft.crop.is_some() {
                                            "A nondestructive crop is active"
                                        } else {
                                            "No crop applied"
                                        })
                                        .size(10.0)
                                        .color(Color32::GRAY),
                                    );
                                }
                                slider(
                                    ui,
                                    "Straighten",
                                    &mut draft.straighten,
                                    -45.0..=45.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                ui.horizontal(|ui| {
                                    if ui.button("90 CCW").clicked() {
                                        draft.rotation = (draft.rotation - 90).rem_euclid(360);
                                        changed = true;
                                        commit = true;
                                    }
                                    if ui.button("90 CW").clicked() {
                                        draft.rotation = (draft.rotation + 90).rem_euclid(360);
                                        changed = true;
                                        commit = true;
                                    }
                                    if ui.button("Flip H").clicked() {
                                        draft.flip_horizontal = !draft.flip_horizontal;
                                        changed = true;
                                        commit = true;
                                    }
                                });
                            });
                        egui::CollapsingHeader::new("Dust & Spot Removal")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if ui
                                        .selectable_label(self.spot_mode, "Repair Brush")
                                        .on_hover_text("Paint over dust or small smudges on the photo")
                                        .clicked()
                                    {
                                        self.spot_mode = !self.spot_mode;
                                        self.crop_mode = false;
                                        self.compare_mode = CompareMode::Edited;
                                    }
                                    if ui
                                        .add_enabled(!draft.spots.is_empty(), egui::Button::new("Undo Dab"))
                                        .clicked()
                                    {
                                        draft.spots.pop();
                                        changed = true;
                                        commit = true;
                                    }
                                    if ui
                                        .add_enabled(!draft.spots.is_empty(), egui::Button::new("Clear"))
                                        .clicked()
                                    {
                                        draft.spots.clear();
                                        changed = true;
                                        commit = true;
                                    }
                                });
                                ui.add(
                                    egui::Slider::new(&mut self.spot_radius, 0.005..=0.12)
                                        .text("Brush size")
                                        .custom_formatter(|value, _| format!("{:.0}%", value * 100.0)),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{} repair dab(s) | drag on the image to paint",
                                        draft.spots.len()
                                    ))
                                    .size(10.0)
                                    .color(Color32::GRAY),
                                );
                            });
                        egui::CollapsingHeader::new("Color Mixer (HSL)")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for index in 0..8 {
                                        if ui
                                            .selectable_label(
                                                self.hsl_band == index,
                                                RichText::new(HSL_NAMES[index])
                                                    .color(HSL_COLORS[index]),
                                            )
                                            .clicked()
                                        {
                                            self.hsl_band = index;
                                        }
                                    }
                                });
                                let band = draft.hsl.band_mut(self.hsl_band);
                                slider(
                                    ui,
                                    "Hue",
                                    &mut band.hue,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Saturation",
                                    &mut band.saturation,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Luminance",
                                    &mut band.luminance,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                            });
                        egui::CollapsingHeader::new("Tone Curve")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for (index, name) in
                                        ["Master", "Red", "Green", "Blue"].into_iter().enumerate()
                                    {
                                        if ui
                                            .selectable_label(self.curve_channel == index, name)
                                            .clicked()
                                        {
                                            self.curve_channel = index;
                                        }
                                    }
                                });
                                let curve = match self.curve_channel {
                                    0 => &mut draft.curves.master,
                                    1 => &mut draft.curves.red,
                                    2 => &mut draft.curves.green,
                                    _ => &mut draft.curves.blue,
                                };
                                let (curve_changed, curve_commit) =
                                    tone_curve_editor(ui, curve, self.curve_channel);
                                changed |= curve_changed;
                                commit |= curve_commit;
                                if ui.small_button("Reset this curve").clicked() {
                                    *curve = ToneCurve::default();
                                    changed = true;
                                    commit = true;
                                }
                            });
                        if changed {
                            self.draft = draft.sanitized();
                            self.preview = None;
                        }
                        if commit {
                            self.finish_edit(id);
                        }
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("Copy Edits").clicked() {
                                self.execute(Command::CopyEdits { id });
                            }
                            if ui
                                .add_enabled(
                                    self.workspace.clipboard.is_some(),
                                    egui::Button::new("Paste Edits"),
                                )
                                .clicked()
                                && self.execute_and_autosave(Command::PasteEdits {
                                    ids: self.selected_photo_ids(),
                                })
                            {
                                for selected in self.selected_photo_ids() {
                                    self.thumbnails.remove(&selected);
                                }
                                self.draft_id = None;
                                self.sync_draft();
                            }
                            if ui
                                .button(
                                    RichText::new("Reset...").color(Color32::from_rgb(245, 150, 130)),
                                )
                                .clicked()
                            {
                                self.reset_confirmation = true;
                            }
                        });
                        ui.separator();
                        self.presets_ui(ui, id);
                        self.history_ui(ui, id);
                    });
            });
    }

    fn histogram_ui(&self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(CANVAS)
            .stroke(Stroke::new(1.0, Color32::from_gray(55)))
            .corner_radius(7.0)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("HISTOGRAM")
                            .strong()
                            .size(10.0)
                            .color(Color32::GRAY),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("RGB + luminance")
                                .size(9.0)
                                .color(Color32::from_gray(105)),
                        );
                    });
                });
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 112.0), Sense::hover());
                if let Some(histogram) = &self.histogram {
                    paint_histogram(ui, rect, histogram);
                }
            });
    }

    fn photo_details_ui(&self, ui: &mut egui::Ui, id: u64) {
        egui::CollapsingHeader::new("Photo Details")
            .default_open(true)
            .show(ui, |ui| {
                let Ok(photo) = self.workspace.project.photo(id) else {
                    return;
                };
                let metadata = &photo.metadata;
                detail_row(
                    ui,
                    "Camera",
                    metadata
                        .camera_model
                        .as_deref()
                        .or(metadata.camera_make.as_deref())
                        .unwrap_or("--"),
                );
                detail_row(ui, "Lens", metadata.lens.as_deref().unwrap_or("--"));
                detail_row(
                    ui,
                    "Capture",
                    &format!(
                        "{}  |  {}  |  {}  |  {}",
                        metadata
                            .iso
                            .map_or_else(|| "ISO --".into(), |iso| format!("ISO {iso}")),
                        metadata
                            .focal_length_mm
                            .map_or_else(|| "-- mm".into(), |value| format!("{value:.0} mm")),
                        metadata
                            .aperture
                            .map_or_else(|| "f/--".into(), |value| format!("f/{value:.1}")),
                        format_shutter(metadata.shutter_seconds),
                    ),
                );
                if let Some(captured) = &metadata.captured_at {
                    detail_row(ui, "Date", captured);
                }
            });
    }

    fn presets_ui(&mut self, ui: &mut egui::Ui, id: u64) {
        egui::CollapsingHeader::new("Presets")
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.preset_name)
                            .hint_text("Preset name")
                            .desired_width(150.0),
                    );
                    if ui
                        .add_enabled(
                            !self.preset_name.trim().is_empty(),
                            egui::Button::new("Save Current"),
                        )
                        .clicked()
                        && self.execute_and_autosave(Command::SavePreset {
                            name: self.preset_name.trim().to_owned(),
                            from_id: id,
                        })
                    {
                        self.preset_name.clear();
                    }
                });
                ui.label(
                    RichText::new("Presets save development settings, not crop or rotation.")
                        .size(9.0)
                        .color(Color32::GRAY),
                );
                let presets: Vec<_> = self
                    .workspace
                    .project
                    .presets
                    .iter()
                    .map(|preset| (preset.id, preset.name.clone()))
                    .collect();
                for (preset_id, name) in presets {
                    ui.horizontal(|ui| {
                        if ui.button(&name).clicked()
                            && self.execute_and_autosave(Command::ApplyPreset {
                                preset_id,
                                ids: self.selected_photo_ids(),
                            })
                        {
                            for selected in self.selected_photo_ids() {
                                self.thumbnails.remove(&selected);
                            }
                            self.draft_id = None;
                            self.sync_draft();
                        }
                        if ui
                            .small_button("x")
                            .on_hover_text("Delete preset")
                            .clicked()
                        {
                            self.execute_and_autosave(Command::DeletePreset { id: preset_id });
                        }
                    });
                }
                if self.workspace.project.presets.is_empty() {
                    ui.label(RichText::new("No saved presets yet").color(Color32::GRAY));
                }
            });
    }

    fn history_ui(&mut self, ui: &mut egui::Ui, id: u64) {
        egui::CollapsingHeader::new("Edit History")
            .default_open(false)
            .show(ui, |ui| {
                let Some(photo) = self.workspace.project.photo(id).ok() else {
                    return;
                };
                let cursor = photo.history_cursor;
                let entries: Vec<_> = photo
                    .history
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| (index, entry.label.clone()))
                    .collect();
                for (index, label) in entries.into_iter().rev().take(20) {
                    let marker = if index == cursor { ">" } else { " " };
                    if ui
                        .selectable_label(index == cursor, format!("{marker}  {label}"))
                        .clicked()
                        && self.execute_and_autosave(Command::HistoryJump { id, index })
                    {
                        self.draft_id = None;
                        self.sync_draft();
                    }
                }
            });
    }

    fn canvas(&mut self, root: &mut egui::Ui) {
        let context = root.ctx().clone();
        self.ensure_preview(&context);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(CANVAS).inner_margin(16))
            .show(root, |ui| {
                if let Some(texture) = self.preview.clone() {
                    let available = ui.available_size();
                    let (rect, response) =
                        ui.allocate_exact_size(available, Sense::click_and_drag());
                    if response.dragged() && !self.crop_mode && !self.spot_mode {
                        self.pan += response.drag_delta();
                    }
                    if response.hovered() && !self.crop_mode && !self.spot_mode {
                        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
                        if scroll.abs() > 0.1 {
                            self.zoom = (self.zoom * (scroll * 0.0018).exp()).clamp(0.25, 8.0);
                        }
                    }
                    if self.compare_mode == CompareMode::SideBySide
                        && let Some(original) = self.original_preview.clone()
                    {
                        let gap = 10.0;
                        let half = (rect.width() - gap) * 0.5;
                        let before_rect =
                            Rect::from_min_size(rect.min, Vec2::new(half, rect.height()));
                        let after_rect = Rect::from_min_size(
                            Pos2::new(before_rect.right() + gap, rect.top()),
                            Vec2::new(half, rect.height()),
                        );
                        ui.painter()
                            .rect_filled(before_rect, 6.0, Color32::from_rgb(18, 20, 23));
                        ui.painter()
                            .rect_filled(after_rect, 6.0, Color32::from_rgb(18, 20, 23));
                        let before_size =
                            fit_size(original.size_vec2(), before_rect.size()) * self.zoom;
                        let after_size = preview_fit_size(
                            self.preview_layout_size,
                            texture.size_vec2(),
                            after_rect.size(),
                        ) * self.zoom;
                        let before_image =
                            Rect::from_center_size(before_rect.center() + self.pan, before_size);
                        let after_image =
                            Rect::from_center_size(after_rect.center() + self.pan, after_size);
                        paint_texture(ui, before_rect, &original, before_image);
                        paint_texture(ui, after_rect, &texture, after_image);
                        compare_label(ui, before_rect, "ORIGINAL");
                        compare_label(ui, after_rect, "EDITED");
                        return;
                    }
                    let base = preview_fit_size(
                        self.preview_layout_size,
                        texture.size_vec2(),
                        rect.size(),
                    );
                    let size = base * self.zoom;
                    let image_rect = Rect::from_center_size(rect.center() + self.pan, size);
                    ui.painter().with_clip_rect(rect).image(
                        texture.id(),
                        image_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    if self.crop_mode {
                        crop_interaction(
                            ui,
                            &response,
                            image_rect,
                            &mut self.crop_draft,
                            &mut self.crop_drag,
                        );
                        paint_crop_overlay(ui, rect, image_rect, self.crop_draft);
                    } else if self.spot_mode {
                        if let Some(id) = self.workspace.project.selected {
                            let (changed, commit) = spot_interaction(
                                &response,
                                image_rect,
                                &mut self.draft.spots,
                                self.spot_radius,
                                &mut self.spot_stroke_start,
                            );
                            if changed {
                                self.preview = None;
                                ui.ctx().request_repaint();
                            }
                            if commit {
                                self.draft = self.draft.clone().sanitized();
                                self.finish_edit(id);
                            }
                        }
                        paint_spot_overlay(
                            ui,
                            rect,
                            image_rect,
                            &self.draft.spots,
                            self.spot_radius,
                        );
                    } else if self.zoom > 1.01 {
                        ui.painter().text(
                            rect.left_bottom() + Vec2::new(8.0, -8.0),
                            egui::Align2::LEFT_BOTTOM,
                            "Drag to pan | Scroll to zoom",
                            egui::FontId::proportional(11.0),
                            Color32::from_gray(150),
                        );
                    }
                } else {
                    let available = ui.available_rect_before_wrap();
                    ui.allocate_rect(available, Sense::hover());
                    let content = Rect::from_center_size(
                        available.center(),
                        Vec2::new(available.width().min(560.0), 190.0),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(content)
                            .layout(egui::Layout::top_down(egui::Align::Center)),
                        |ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("L U M E N").size(12.0).strong().color(ACCENT));
                            ui.add_space(14.0);
                            ui.label(
                                RichText::new("Bring a shoot into focus")
                                    .size(26.0)
                                    .color(Color32::from_gray(226)),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("Drop photos here, or start by choosing a shoot.")
                                    .size(13.0)
                                    .color(Color32::from_gray(145)),
                            );
                            ui.add_space(16.0);
                            ui.horizontal(|ui| {
                                if ui.button("Choose Photos").clicked() {
                                    self.import_dialog();
                                }
                                if ui.button("Open Catalog").clicked() {
                                    self.open_catalog();
                                }
                            });
                        },
                    );
                }
            });
    }

    fn status_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(28.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(19, 20, 23))
                    .inner_margin(6),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.status).size(12.0).color(if self.error {
                        Color32::from_rgb(244, 122, 110)
                    } else {
                        Color32::LIGHT_GRAY
                    }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(photo) = self.workspace.project.selected_photo() {
                            ui.label(
                                RichText::new(format!(
                                    "{} x {}  |  {}",
                                    photo.width,
                                    photo.height,
                                    photo.format.to_uppercase()
                                ))
                                .size(12.0)
                                .color(Color32::GRAY),
                            );
                        }
                    });
                });
            });
    }

    fn confirmation_window(&mut self, context: &egui::Context) {
        if !self.reset_confirmation {
            return;
        }
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Reset all edits?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label(format!(
                    "Reset {} selected photo(s)? This adds a reversible event to each history.",
                    self.selected_photo_ids().len()
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(
                            RichText::new("Reset all edits")
                                .color(Color32::from_rgb(245, 150, 130)),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if cancel {
            self.reset_confirmation = false;
        }
        if confirm {
            self.reset_confirmation = false;
            let ids = self.selected_photo_ids();
            if !ids.is_empty() && self.execute_and_autosave(Command::Reset { ids: ids.clone() }) {
                for id in ids {
                    self.thumbnails.remove(&id);
                }
                self.draft_id = None;
                self.sync_draft();
            }
        }
    }

    fn remove_confirmation_window(&mut self, context: &egui::Context) {
        if !self.remove_confirmation {
            return;
        }
        let ids = self.selected_photo_ids();
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Remove from catalog?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label(format!(
                    "Remove {} selected photo(s) from this catalog?",
                    ids.len()
                ));
                ui.label(
                    RichText::new("Original photo files will not be deleted.")
                        .strong()
                        .color(ACCENT),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(
                            RichText::new("Remove Photos").color(Color32::from_rgb(245, 150, 130)),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if cancel {
            self.remove_confirmation = false;
        }
        if confirm {
            self.remove_confirmation = false;
            let active_batch = self.active_batch;
            if !ids.is_empty() && self.execute_and_autosave(Command::Remove { ids }) {
                let active_batch =
                    active_batch.filter(|batch_id| self.workspace.project.batch(*batch_id).is_ok());
                self.reset_catalog_view(active_batch.is_none());
                self.active_batch = active_batch;
            }
        }
    }

    fn catalog_switch_confirmation_window(&mut self, context: &egui::Context) {
        let Some(action) = self.pending_catalog_switch.clone() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Leave unsaved catalog?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label(format!(
                    "This catalog contains {} photo(s) and has not been saved.",
                    self.workspace.project.photos.len()
                ));
                ui.label("Save it first, or continue and leave this catalog behind.");
                ui.label(
                    RichText::new("Original photo files will not be deleted.").color(Color32::GRAY),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Save Current...").clicked() {
                        self.save_catalog(true);
                        if self.workspace.catalog_path.is_some() {
                            confirm = true;
                        }
                    }
                    if ui
                        .button(
                            RichText::new("Continue Without Saving")
                                .color(Color32::from_rgb(245, 150, 130)),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if cancel {
            self.pending_catalog_switch = None;
        }
        if confirm {
            self.pending_catalog_switch = None;
            self.apply_catalog_switch(action);
        }
    }

    fn rename_batch_window(&mut self, context: &egui::Context) {
        if self.rename_batch.is_none() {
            return;
        }
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Rename Shoot")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                if let Some((_, name)) = self.rename_batch.as_mut() {
                    ui.label("Shoot name");
                    let response = ui.text_edit_singleline(name);
                    if response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    {
                        save = true;
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(RichText::new("Save Name").color(ACCENT))
                        .clicked()
                    {
                        save = true;
                    }
                });
            });
        if cancel {
            self.rename_batch = None;
        }
        if save
            && let Some((id, name)) = self.rename_batch.take()
            && !self.execute_and_autosave(Command::RenameBatch {
                id,
                name: name.clone(),
            })
        {
            self.rename_batch = Some((id, name));
        }
    }

    fn export_window(&mut self, context: &egui::Context) {
        if !self.export_open {
            return;
        }
        let ids = self.selected_photo_ids();
        let mut close = false;
        let mut export = false;
        egui::Window::new(format!("Export {} Photo(s)", ids.len()))
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label(
                    RichText::new("FILE TYPE")
                        .strong()
                        .size(11.0)
                        .color(Color32::GRAY),
                );
                ui.horizontal(|ui| {
                    for (format, label) in [
                        (ExportFormat::Jpeg, "JPEG"),
                        (ExportFormat::Png, "PNG"),
                        (ExportFormat::Tiff, "TIFF"),
                        (ExportFormat::Webp, "WebP"),
                    ] {
                        ui.selectable_value(&mut self.export_format, format, label);
                    }
                });
                if self.export_format == ExportFormat::Jpeg {
                    ui.add(
                        egui::Slider::new(&mut self.export_quality, 1..=100).text("JPEG Quality"),
                    );
                } else if self.export_format == ExportFormat::Webp {
                    ui.label(
                        RichText::new("WebP export is lossless in the current encoder.")
                            .size(10.0)
                            .color(Color32::GRAY),
                    );
                }
                ui.add(
                    egui::Slider::new(&mut self.export_max_size, 0..=8000)
                        .text("Maximum long edge")
                        .suffix(" px"),
                );
                if self.export_max_size == 0 {
                    ui.label(
                        RichText::new("0 uses full resolution")
                            .size(10.0)
                            .color(Color32::GRAY),
                    );
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Choose Export Folder...").clicked() {
                        self.export_directory = rfd::FileDialog::new().pick_folder();
                    }
                    if let Some(directory) = &self.export_directory {
                        ui.label(shorten(&directory.display().to_string(), 38));
                    } else {
                        ui.label(RichText::new("No folder selected").color(Color32::GRAY));
                    }
                });
                let estimate = estimate_export_bytes(
                    &self.workspace,
                    &ids,
                    self.export_format,
                    self.export_quality,
                    self.export_max_size,
                );
                ui.add_space(4.0);
                ui.label(format!(
                    "Estimated output: {} total | about {} per photo",
                    format_bytes(estimate),
                    format_bytes(estimate / ids.len().max(1) as u64)
                ));
                ui.label(
                    RichText::new("Estimate varies with image detail and compression.")
                        .size(10.0)
                        .color(Color32::GRAY),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui
                        .add_enabled(
                            self.export_directory.is_some() && !ids.is_empty(),
                            egui::Button::new(RichText::new("Export").color(ACCENT)),
                        )
                        .clicked()
                    {
                        export = true;
                    }
                });
            });
        if close {
            self.export_open = false;
        }
        if export
            && let Some(directory) = self.export_directory.clone()
            && self.execute(Command::ExportBatch {
                ids,
                directory,
                format: self.export_format,
                max_size: (self.export_max_size > 0).then_some(self.export_max_size),
                quality: self.export_quality,
            })
        {
            self.export_open = false;
        }
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

impl Histogram {
    fn from_image(image: &DynamicImage) -> Self {
        let mut histogram = Self {
            red: [0; 256],
            green: [0; 256],
            blue: [0; 256],
            luma: [0; 256],
        };
        let rgba = image.to_rgba8();
        for pixel in rgba.pixels().step_by(2) {
            histogram.red[pixel[0] as usize] += 1;
            histogram.green[pixel[1] as usize] += 1;
            histogram.blue[pixel[2] as usize] += 1;
            let luma =
                (pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722)
                    .round() as usize;
            histogram.luma[luma.min(255)] += 1;
        }
        histogram
    }
}

fn paint_histogram(ui: &egui::Ui, rect: Rect, histogram: &Histogram) {
    let painter = ui.painter_at(rect);
    for fraction in [0.25, 0.5, 0.75] {
        let x = rect.left() + rect.width() * fraction;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_gray(33)),
        );
    }
    let peak = histogram
        .luma
        .iter()
        .chain(histogram.red.iter())
        .chain(histogram.green.iter())
        .chain(histogram.blue.iter())
        .copied()
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    for (values, color, width) in [
        (&histogram.luma, Color32::from_white_alpha(115), 1.5),
        (
            &histogram.red,
            Color32::from_rgba_unmultiplied(238, 77, 72, 175),
            1.15,
        ),
        (
            &histogram.green,
            Color32::from_rgba_unmultiplied(78, 211, 115, 175),
            1.15,
        ),
        (
            &histogram.blue,
            Color32::from_rgba_unmultiplied(82, 137, 240, 185),
            1.15,
        ),
    ] {
        let points: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                Pos2::new(
                    rect.left() + index as f32 / 255.0 * rect.width(),
                    rect.bottom() - (*value as f32 / peak).sqrt() * rect.height(),
                )
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(width, color)));
    }
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [58.0, 16.0],
            egui::Label::new(RichText::new(label).size(10.0).color(Color32::GRAY)),
        );
        ui.label(RichText::new(value).size(10.0));
    });
}

fn format_shutter(seconds: Option<f32>) -> String {
    let Some(seconds) = seconds.filter(|value| *value > 0.0) else {
        return "-- s".into();
    };
    if seconds >= 1.0 {
        format!("{seconds:.1} s")
    } else {
        format!("1/{:.0} s", 1.0 / seconds)
    }
}

fn grade_swatch(ui: &mut egui::Ui, grade: ColorGrade) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 11.0, hue_color(grade.hue, grade.saturation));
    ui.painter().circle_stroke(
        rect.center(),
        11.0,
        Stroke::new(1.0, Color32::from_gray(110)),
    );
}

fn hue_color(hue: f32, saturation: f32) -> Color32 {
    let h = hue.rem_euclid(360.0) / 60.0;
    let s = saturation.clamp(0.0, 100.0) / 100.0;
    let x = 1.0 - (h.rem_euclid(2.0) - 1.0).abs();
    let rgb = match h as i32 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    };
    let mix = |channel: f32| ((0.38 + channel * 0.62) * s + 0.42 * (1.0 - s)) * 255.0;
    Color32::from_rgb(mix(rgb[0]) as u8, mix(rgb[1]) as u8, mix(rgb[2]) as u8)
}

fn paint_texture(ui: &egui::Ui, clip: Rect, texture: &TextureHandle, image: Rect) {
    ui.painter().with_clip_rect(clip).image(
        texture.id(),
        image,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn compare_label(ui: &egui::Ui, rect: Rect, label: &str) {
    let badge = Rect::from_min_size(rect.min + Vec2::splat(10.0), Vec2::new(78.0, 24.0));
    ui.painter()
        .rect_filled(badge, 5.0, Color32::from_black_alpha(185));
    ui.painter().text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(10.0),
        if label == "EDITED" {
            ACCENT
        } else {
            Color32::LIGHT_GRAY
        },
    );
}

fn spot_interaction(
    response: &egui::Response,
    image: Rect,
    spots: &mut Vec<SpotRemoval>,
    radius: f32,
    stroke_start: &mut Option<usize>,
) -> (bool, bool) {
    let mut changed = false;
    let mut commit = false;
    let point = |position: Pos2| {
        image.contains(position).then_some(SpotRemoval {
            x: ((position.x - image.left()) / image.width()).clamp(0.0, 1.0),
            y: ((position.y - image.top()) / image.height()).clamp(0.0, 1.0),
            radius,
            opacity: 1.0,
        })
    };
    let add = |spots: &mut Vec<SpotRemoval>, spot: SpotRemoval| {
        let spaced = spots.last().is_none_or(|last| {
            let distance = ((last.x - spot.x).powi(2) + (last.y - spot.y).powi(2)).sqrt();
            distance >= radius * 0.55
        });
        if spaced && spots.len() < 512 {
            spots.push(spot);
            true
        } else {
            false
        }
    };
    if response.drag_started() {
        *stroke_start = Some(spots.len());
    }
    if (response.drag_started() || response.dragged())
        && let Some(position) = response.interact_pointer_pos()
        && let Some(spot) = point(position)
    {
        changed |= add(spots, spot);
    }
    if response.drag_stopped()
        && let Some(start) = stroke_start.take()
    {
        commit = spots.len() > start;
    }
    if response.clicked()
        && let Some(position) = response.interact_pointer_pos()
        && let Some(spot) = point(position)
    {
        changed |= add(spots, spot);
        commit = changed;
    }
    (changed, commit)
}

fn paint_spot_overlay(
    ui: &egui::Ui,
    canvas: Rect,
    image: Rect,
    spots: &[SpotRemoval],
    brush_radius: f32,
) {
    let painter = ui.painter().with_clip_rect(canvas.intersect(image));
    let scale = image.width().min(image.height());
    for spot in spots {
        let center = Pos2::new(
            image.left() + spot.x * image.width(),
            image.top() + spot.y * image.height(),
        );
        painter.circle_stroke(
            center,
            spot.radius * scale,
            Stroke::new(1.2, Color32::from_rgba_unmultiplied(255, 255, 255, 150)),
        );
    }
    if let Some(position) = ui.ctx().pointer_hover_pos()
        && image.contains(position)
    {
        painter.circle_stroke(position, brush_radius * scale, Stroke::new(1.5, ACCENT));
    }
}

fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    changed: &mut bool,
    commit: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [78.0, 18.0],
            egui::Label::new(RichText::new(label).size(10.5)),
        );
        let response = ui.add(
            egui::Slider::new(value, range)
                .show_value(true)
                .smart_aim(false),
        );
        *changed |= response.changed();
        *commit |= response.drag_stopped() || (response.changed() && !response.dragged());
        if ui
            .add_enabled(value.abs() > f32::EPSILON, egui::Button::new("0").small())
            .on_hover_text(format!("Reset {label}"))
            .clicked()
        {
            *value = 0.0;
            *changed = true;
            *commit = true;
        }
    });
}

fn crop_screen_rect(image: Rect, crop: CropRect) -> Rect {
    Rect::from_min_max(
        Pos2::new(
            image.left() + crop.x * image.width(),
            image.top() + crop.y * image.height(),
        ),
        Pos2::new(
            image.left() + (crop.x + crop.width) * image.width(),
            image.top() + (crop.y + crop.height) * image.height(),
        ),
    )
}

fn crop_handle_at(position: Pos2, crop: Rect) -> Option<CropHandle> {
    let threshold = 14.0;
    let near = |point: Pos2| point.distance(position) <= threshold;
    if near(crop.left_top()) {
        Some(CropHandle::TopLeft)
    } else if near(crop.right_top()) {
        Some(CropHandle::TopRight)
    } else if near(crop.left_bottom()) {
        Some(CropHandle::BottomLeft)
    } else if near(crop.right_bottom()) {
        Some(CropHandle::BottomRight)
    } else if (position.x - crop.left()).abs() <= threshold
        && (crop.top()..=crop.bottom()).contains(&position.y)
    {
        Some(CropHandle::Left)
    } else if (position.x - crop.right()).abs() <= threshold
        && (crop.top()..=crop.bottom()).contains(&position.y)
    {
        Some(CropHandle::Right)
    } else if (position.y - crop.top()).abs() <= threshold
        && (crop.left()..=crop.right()).contains(&position.x)
    {
        Some(CropHandle::Top)
    } else if (position.y - crop.bottom()).abs() <= threshold
        && (crop.left()..=crop.right()).contains(&position.x)
    {
        Some(CropHandle::Bottom)
    } else if crop.contains(position) {
        Some(CropHandle::Move)
    } else {
        None
    }
}

fn crop_interaction(
    ui: &egui::Ui,
    response: &egui::Response,
    image: Rect,
    crop: &mut CropRect,
    drag: &mut Option<CropDrag>,
) {
    if response.drag_started()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(handle) = crop_handle_at(pointer, crop_screen_rect(image, *crop))
    {
        *drag = Some(CropDrag {
            handle,
            start: *crop,
            pointer,
        });
    }
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(active) = *drag
    {
        let dx = (pointer.x - active.pointer.x) / image.width().max(1.0);
        let dy = (pointer.y - active.pointer.y) / image.height().max(1.0);
        let mut left = active.start.x;
        let mut top = active.start.y;
        let mut right = active.start.x + active.start.width;
        let mut bottom = active.start.y + active.start.height;
        const MINIMUM: f32 = 0.025;
        match active.handle {
            CropHandle::Move => {
                let width = right - left;
                let height = bottom - top;
                left = (left + dx).clamp(0.0, 1.0 - width);
                top = (top + dy).clamp(0.0, 1.0 - height);
                right = left + width;
                bottom = top + height;
            }
            CropHandle::Left | CropHandle::TopLeft | CropHandle::BottomLeft => {
                left = (left + dx).clamp(0.0, right - MINIMUM);
            }
            CropHandle::Right | CropHandle::TopRight | CropHandle::BottomRight => {
                right = (right + dx).clamp(left + MINIMUM, 1.0);
            }
            _ => {}
        }
        match active.handle {
            CropHandle::Top | CropHandle::TopLeft | CropHandle::TopRight => {
                top = (top + dy).clamp(0.0, bottom - MINIMUM);
            }
            CropHandle::Bottom | CropHandle::BottomLeft | CropHandle::BottomRight => {
                bottom = (bottom + dy).clamp(top + MINIMUM, 1.0);
            }
            _ => {}
        }
        *crop = CropRect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
        .sanitized();
        ui.ctx().request_repaint();
    }
    if response.drag_stopped() {
        *drag = None;
    }
}

fn paint_crop_overlay(ui: &egui::Ui, canvas: Rect, image: Rect, crop: CropRect) {
    let crop = crop_screen_rect(image, crop);
    let painter = ui.painter().with_clip_rect(canvas.intersect(image));
    let shade = Color32::from_black_alpha(165);
    for rect in [
        Rect::from_min_max(image.min, Pos2::new(image.right(), crop.top())),
        Rect::from_min_max(Pos2::new(image.left(), crop.bottom()), image.max),
        Rect::from_min_max(
            Pos2::new(image.left(), crop.top()),
            Pos2::new(crop.left(), crop.bottom()),
        ),
        Rect::from_min_max(
            Pos2::new(crop.right(), crop.top()),
            Pos2::new(image.right(), crop.bottom()),
        ),
    ] {
        painter.rect_filled(rect, 0.0, shade);
    }
    painter.rect_stroke(
        crop,
        0.0,
        Stroke::new(2.0, Color32::WHITE),
        egui::StrokeKind::Inside,
    );
    for fraction in [1.0 / 3.0, 2.0 / 3.0] {
        let x = crop.left() + crop.width() * fraction;
        let y = crop.top() + crop.height() * fraction;
        painter.line_segment(
            [Pos2::new(x, crop.top()), Pos2::new(x, crop.bottom())],
            Stroke::new(1.0, Color32::from_white_alpha(150)),
        );
        painter.line_segment(
            [Pos2::new(crop.left(), y), Pos2::new(crop.right(), y)],
            Stroke::new(1.0, Color32::from_white_alpha(150)),
        );
    }
    let handles = [
        crop.left_top(),
        crop.right_top(),
        crop.left_bottom(),
        crop.right_bottom(),
        Pos2::new(crop.center().x, crop.top()),
        Pos2::new(crop.center().x, crop.bottom()),
        Pos2::new(crop.left(), crop.center().y),
        Pos2::new(crop.right(), crop.center().y),
    ];
    for center in handles {
        painter.rect_filled(
            Rect::from_center_size(center, Vec2::splat(9.0)),
            1.0,
            Color32::WHITE,
        );
        painter.rect_stroke(
            Rect::from_center_size(center, Vec2::splat(9.0)),
            1.0,
            Stroke::new(1.0, Color32::BLACK),
            egui::StrokeKind::Inside,
        );
    }
}

fn set_crop_aspect(crop: &mut CropRect, output_ratio: f32, source_ratio: f32) {
    let normalized_ratio = output_ratio / source_ratio.max(0.01);
    let center = (crop.x + crop.width * 0.5, crop.y + crop.height * 0.5);
    let mut width = crop.width;
    let mut height = width / normalized_ratio;
    if height > crop.height {
        height = crop.height;
        width = height * normalized_ratio;
    }
    width = width.clamp(0.025, 1.0);
    height = height.clamp(0.025, 1.0);
    crop.x = (center.0 - width * 0.5).clamp(0.0, 1.0 - width);
    crop.y = (center.1 - height * 0.5).clamp(0.0, 1.0 - height);
    crop.width = width;
    crop.height = height;
}

fn estimate_export_bytes(
    workspace: &Workspace,
    ids: &[u64],
    format: ExportFormat,
    quality: u8,
    max_size: u32,
) -> u64 {
    ids.iter()
        .filter_map(|id| workspace.project.photo(*id).ok())
        .map(|photo| {
            let crop = photo.adjustments.crop.unwrap_or_default();
            let mut width = photo.width as f64 * crop.width as f64;
            let mut height = photo.height as f64 * crop.height as f64;
            let long = width.max(height);
            if max_size > 0 && long > max_size as f64 {
                let scale = max_size as f64 / long;
                width *= scale;
                height *= scale;
            }
            format.estimate_bytes((width * height) as u64, quality)
        })
        .sum()
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{:.0} KB", bytes as f64 / 1_000.0)
    }
}

fn tone_curve_editor(ui: &mut egui::Ui, curve: &mut ToneCurve, channel: usize) -> (bool, bool) {
    // Keep the graph and its endpoint handles clear of the inspector scrollbar.
    let desired = Vec2::new((ui.available_width() - 18.0).max(140.0), 176.0);
    let (outer, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let rect = outer.shrink(7.0);
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, 4.0, CANVAS);
    for i in 1..4 {
        let t = i as f32 / 4.0;
        let x = egui::lerp(rect.x_range(), t);
        let y = egui::lerp(rect.y_range(), t);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_gray(43)),
        );
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, Color32::from_gray(43)),
        );
    }
    let curve_color = [
        Color32::WHITE,
        Color32::from_rgb(235, 91, 91),
        Color32::from_rgb(89, 210, 119),
        Color32::from_rgb(94, 139, 240),
    ][channel];
    let to_screen = |point: CurvePoint| {
        Pos2::new(
            rect.left() + point.x * rect.width(),
            rect.bottom() - point.y * rect.height(),
        )
    };
    for pair in curve.points.windows(2) {
        painter.line_segment(
            [to_screen(pair[0]), to_screen(pair[1])],
            Stroke::new(2.0, curve_color),
        );
    }
    for point in &curve.points {
        painter.circle_filled(to_screen(*point), 4.0, curve_color);
        painter.circle_stroke(to_screen(*point), 5.0, Stroke::new(1.0, Color32::BLACK));
    }
    let mut changed = false;
    if response.clicked()
        && let Some(position) = response.interact_pointer_pos()
    {
        curve.points.push(CurvePoint {
            x: ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
            y: ((rect.bottom() - position.y) / rect.height()).clamp(0.0, 1.0),
        });
        *curve = curve.clone().sanitized();
        changed = true;
    }
    if response.dragged()
        && let Some(position) = response.interact_pointer_pos()
    {
        let pointer = CurvePoint {
            x: ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
            y: ((rect.bottom() - position.y) / rect.height()).clamp(0.0, 1.0),
        };
        if let Some((index, _)) = curve.points.iter().enumerate().min_by(|(_, a), (_, b)| {
            let da = (a.x - pointer.x).powi(2) + (a.y - pointer.y).powi(2);
            let db = (b.x - pointer.x).powi(2) + (b.y - pointer.y).powi(2);
            da.total_cmp(&db)
        }) {
            curve.points[index].y = pointer.y;
            if index > 0 && index + 1 < curve.points.len() {
                curve.points[index].x = pointer.x.clamp(
                    curve.points[index - 1].x + 0.005,
                    curve.points[index + 1].x - 0.005,
                );
            }
            changed = true;
        }
    }
    (
        changed,
        response.drag_stopped() || (changed && !response.dragged()),
    )
}

fn load_texture(context: &egui::Context, name: String, image: DynamicImage) -> TextureHandle {
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    context.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
        TextureOptions::LINEAR,
    )
}

fn fit_size(image: Vec2, available: Vec2) -> Vec2 {
    image * (available.x / image.x).min(available.y / image.y).min(1.0)
}

fn preview_fit_size(layout: Option<Vec2>, raster: Vec2, available: Vec2) -> Vec2 {
    fit_size(layout.unwrap_or(raster), available)
}

fn same_preview_geometry(left: &Adjustments, right: &Adjustments) -> bool {
    left.crop == right.crop
        && left.rotation == right.rotation
        && left.straighten == right.straighten
        && left.flip_horizontal == right.flip_horizontal
        && left.flip_vertical == right.flip_vertical
}

fn shorten(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

fn catalog_label(path: &Path) -> String {
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Catalog");
    let parent = path
        .parent()
        .and_then(|value| value.file_name())
        .and_then(|value| value.to_str());
    parent.map_or_else(|| name.to_owned(), |parent| format!("{name}  /  {parent}"))
}

fn current_catalog_name(workspace: &Workspace) -> String {
    workspace
        .catalog_path
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unsaved catalog".into())
}

#[allow(dead_code)]
fn _assert_photo_is_available(_: Photo) {}

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
