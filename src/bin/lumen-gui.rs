#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{collections::HashMap, path::PathBuf};

use eframe::egui::{
    self, Color32, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};
use image::DynamicImage;
use lumen_core::{
    Adjustments, Command, CropRect, CurvePoint, Photo, ToneCurve, Workspace,
    engine::{RenderOptions, decode_photo, render_image, render_photo},
    project::is_supported_image,
};

const ACCENT: Color32 = Color32::from_rgb(232, 177, 72);
const PANEL: Color32 = Color32::from_rgb(25, 27, 31);
const CANVAS: Color32 = Color32::from_rgb(14, 15, 18);
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

fn main() -> eframe::Result {
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
        Box::new(|creation| Ok(Box::new(LumenApp::new(creation)))),
    )
}

struct LumenApp {
    workspace: Workspace,
    preview: Option<TextureHandle>,
    preview_source: Option<(u64, DynamicImage)>,
    preview_id: Option<u64>,
    preview_adjustments: Adjustments,
    thumbnails: HashMap<u64, TextureHandle>,
    draft: Adjustments,
    draft_id: Option<u64>,
    status: String,
    error: bool,
    zoom: f32,
    pan: Vec2,
    hsl_band: usize,
    curve_channel: usize,
    reset_confirmation: bool,
}

impl LumenApp {
    fn new(creation: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = PANEL;
        visuals.window_fill = PANEL;
        visuals.selection.bg_fill = ACCENT;
        visuals.selection.stroke.color = Color32::BLACK;
        creation.egui_ctx.set_visuals(visuals);
        creation.egui_ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = Vec2::new(8.0, 7.0);
            style.spacing.button_padding = Vec2::new(10.0, 6.0);
        });
        Self {
            workspace: Workspace::default(),
            preview: None,
            preview_source: None,
            preview_id: None,
            preview_adjustments: Adjustments::default(),
            thumbnails: HashMap::new(),
            draft: Adjustments::default(),
            draft_id: None,
            status: "Drop photos anywhere, or choose Import Photos".into(),
            error: false,
            zoom: 1.0,
            pan: Vec2::ZERO,
            hsl_band: 0,
            curve_channel: 0,
            reset_confirmation: false,
        }
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
        self.preview_id = None;
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
        if self.execute(Command::Select { id }) {
            self.draft_id = None;
            self.sync_draft();
        }
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
        self.status = "Developing RAW previews and importing photos…".into();
        if self.execute_and_autosave(Command::Import { paths }) {
            self.thumbnails.clear();
            self.draft_id = None;
            self.sync_draft();
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

    fn open_catalog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Lumen catalog", &["lumencatalog"])
            .pick_file()
            && self.execute(Command::Open { path })
        {
            self.thumbnails.clear();
            self.draft_id = None;
            self.sync_draft();
        }
    }

    fn save_catalog(&mut self, save_as: bool) {
        let path = if save_as || self.workspace.catalog_path.is_none() {
            rfd::FileDialog::new()
                .add_filter("Lumen catalog", &["lumencatalog"])
                .set_file_name(format!("{}.lumencatalog", self.workspace.project.name))
                .save_file()
        } else {
            None
        };
        if save_as && path.is_none() {
            return;
        }
        self.execute(Command::Save { path });
    }

    fn export_selected(&mut self) {
        let Some(photo) = self.workspace.project.selected_photo() else {
            return;
        };
        let stem = photo.path.file_stem().unwrap_or_default().to_string_lossy();
        let Some(path) = rfd::FileDialog::new()
            .add_filter("JPEG", &["jpg", "jpeg"])
            .add_filter("PNG", &["png"])
            .add_filter("TIFF", &["tif", "tiff"])
            .add_filter("WebP", &["webp"])
            .set_file_name(format!("{stem}-lumen.jpg"))
            .save_file()
        else {
            return;
        };
        self.execute(Command::Export {
            id: photo.id,
            path,
            max_size: None,
            quality: 92,
        });
    }

    fn ensure_preview(&mut self, context: &egui::Context) {
        let Some(id) = self.workspace.project.selected else {
            self.preview = None;
            return;
        };
        if self.preview.is_some()
            && self.preview_id == Some(id)
            && self.preview_adjustments == self.draft
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
                Ok(source) => self.preview_source = Some((id, source)),
                Err(error) => {
                    self.status = format!("preview failed: {error:#}");
                    self.error = true;
                    return;
                }
            }
        }
        if let Some((_, source)) = &self.preview_source {
            let rendered =
                render_image(source.clone(), self.draft.clone(), RenderOptions::default());
            self.preview = Some(load_texture(context, format!("preview-{id}"), rendered));
            self.preview_id = Some(id);
            self.preview_adjustments = self.draft.clone();
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
    }

    fn select_relative(&mut self, direction: i32) {
        if self.workspace.project.photos.is_empty() {
            return;
        }
        let current = self
            .workspace
            .project
            .selected
            .and_then(|id| {
                self.workspace
                    .project
                    .photos
                    .iter()
                    .position(|photo| photo.id == id)
            })
            .unwrap_or(0) as i32;
        let index =
            (current + direction).clamp(0, self.workspace.project.photos.len() as i32 - 1) as usize;
        self.select(self.workspace.project.photos[index].id);
    }

    fn toolbar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(14, 9)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("LUMEN").strong().size(17.0).color(ACCENT));
                    ui.separator();
                    if ui.button("Import Photos").clicked() {
                        self.import_dialog();
                    }
                    if ui.button("Open Catalog").clicked() {
                        self.open_catalog();
                    }
                    if ui.button("Save").clicked() {
                        self.save_catalog(false);
                    }
                    ui.separator();
                    let (can_back, can_forward) = self
                        .workspace
                        .project
                        .selected_photo()
                        .map(|photo| (photo.can_history_back(), photo.can_history_forward()))
                        .unwrap_or_default();
                    if ui
                        .add_enabled(can_back, egui::Button::new("← Edit"))
                        .on_hover_text("Step backward in persistent edit history (Cmd/Ctrl+Z)")
                        .clicked()
                        && let Some(id) = self.workspace.project.selected
                        && self.execute_and_autosave(Command::HistoryBack { id })
                    {
                        self.draft_id = None;
                        self.sync_draft();
                    }
                    if ui
                        .add_enabled(can_forward, egui::Button::new("Edit →"))
                        .on_hover_text("Step forward in persistent edit history (Cmd/Ctrl+Shift+Z)")
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
                    if ui.button("−").clicked() {
                        self.zoom = (self.zoom / 1.25).max(0.25);
                    }
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    if ui.button("+").clicked() {
                        self.zoom = (self.zoom * 1.25).min(8.0);
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.workspace.project.selected.is_some(),
                            egui::Button::new("Export"),
                        )
                        .clicked()
                    {
                        self.export_selected();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let catalog = self
                            .workspace
                            .catalog_path
                            .as_ref()
                            .and_then(|path| path.file_name())
                            .map(|name| name.to_string_lossy())
                            .unwrap_or_else(|| "Unsaved catalog".into());
                        ui.label(RichText::new(catalog).color(Color32::GRAY));
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
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("PHOTOS")
                            .strong()
                            .size(12.0)
                            .color(Color32::GRAY),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(self.workspace.project.photos.len().to_string());
                    });
                });
                ui.separator();
                let photos: Vec<_> = self
                    .workspace
                    .project
                    .photos
                    .iter()
                    .map(|photo| (photo.id, photo.name.clone(), photo.is_raw()))
                    .collect();
                if photos.is_empty() {
                    ui.label(RichText::new("Your photos will appear here").color(Color32::GRAY));
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (id, name, raw) in photos {
                            self.ensure_thumbnail(&context, id);
                            let selected = self.workspace.project.selected == Some(id);
                            let frame = egui::Frame::new()
                                .fill(if selected {
                                    Color32::from_rgb(55, 48, 33)
                                } else {
                                    CANVAS
                                })
                                .stroke(if selected {
                                    Stroke::new(2.0, ACCENT)
                                } else {
                                    Stroke::new(1.0, Color32::from_gray(50))
                                })
                                .corner_radius(5.0)
                                .inner_margin(6);
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
                                    if raw {
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new("RAW")
                                                        .size(9.0)
                                                        .strong()
                                                        .color(ACCENT),
                                                );
                                            },
                                        );
                                    }
                                });
                            });
                            if inner.response.interact(Sense::click()).clicked() {
                                self.select(id);
                            }
                            ui.add_space(5.0);
                        }
                    });
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
                                let mut enabled = draft.crop.is_some();
                                if ui.checkbox(&mut enabled, "Enable crop").changed() {
                                    draft.crop = enabled.then(CropRect::default);
                                    changed = true;
                                    commit = true;
                                }
                                if let Some(mut crop) = draft.crop {
                                    slider(
                                        ui,
                                        "Left",
                                        &mut crop.x,
                                        0.0..=0.99,
                                        &mut changed,
                                        &mut commit,
                                    );
                                    slider(
                                        ui,
                                        "Top",
                                        &mut crop.y,
                                        0.0..=0.99,
                                        &mut changed,
                                        &mut commit,
                                    );
                                    slider(
                                        ui,
                                        "Width",
                                        &mut crop.width,
                                        0.01..=1.0,
                                        &mut changed,
                                        &mut commit,
                                    );
                                    slider(
                                        ui,
                                        "Height",
                                        &mut crop.height,
                                        0.01..=1.0,
                                        &mut changed,
                                        &mut commit,
                                    );
                                    ui.horizontal(|ui| {
                                        if ui.small_button("1:1").clicked() {
                                            crop.width = crop.width.min(crop.height);
                                            crop.height = crop.width;
                                            changed = true;
                                            commit = true;
                                        }
                                        if ui.small_button("4:5").clicked() {
                                            crop.height = (crop.width * 1.25).min(1.0 - crop.y);
                                            changed = true;
                                            commit = true;
                                        }
                                        if ui.small_button("16:9").clicked() {
                                            crop.height =
                                                (crop.width * 9.0 / 16.0).min(1.0 - crop.y);
                                            changed = true;
                                            commit = true;
                                        }
                                    });
                                    draft.crop = Some(crop);
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
                                    if ui.button("↺ 90°").clicked() {
                                        draft.rotation = (draft.rotation - 90).rem_euclid(360);
                                        changed = true;
                                        commit = true;
                                    }
                                    if ui.button("90° ↻").clicked() {
                                        draft.rotation = (draft.rotation + 90).rem_euclid(360);
                                        changed = true;
                                        commit = true;
                                    }
                                    if ui.button("Flip ↔").clicked() {
                                        draft.flip_horizontal = !draft.flip_horizontal;
                                        changed = true;
                                        commit = true;
                                    }
                                });
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
                                && self.execute_and_autosave(Command::PasteEdits { ids: vec![id] })
                            {
                                self.draft_id = None;
                                self.sync_draft();
                            }
                            if ui
                                .button(
                                    RichText::new("Reset…").color(Color32::from_rgb(245, 150, 130)),
                                )
                                .clicked()
                            {
                                self.reset_confirmation = true;
                            }
                        });
                        ui.separator();
                        self.history_ui(ui, id);
                    });
            });
    }

    fn history_ui(&mut self, ui: &mut egui::Ui, id: u64) {
        ui.label(
            RichText::new("EDIT HISTORY")
                .strong()
                .size(11.0)
                .color(Color32::GRAY),
        );
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
        for (index, label) in entries.into_iter().rev().take(12) {
            let marker = if index == cursor { "●" } else { "○" };
            if ui
                .selectable_label(index == cursor, format!("{marker}  {label}"))
                .clicked()
                && self.execute_and_autosave(Command::HistoryJump { id, index })
            {
                self.draft_id = None;
                self.sync_draft();
            }
        }
    }

    fn canvas(&mut self, root: &mut egui::Ui) {
        let context = root.ctx().clone();
        self.ensure_preview(&context);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(CANVAS).inner_margin(16))
            .show(root, |ui| {
                if let Some(texture) = &self.preview {
                    let available = ui.available_size();
                    let (rect, response) =
                        ui.allocate_exact_size(available, Sense::click_and_drag());
                    if response.dragged() {
                        self.pan += response.drag_delta();
                    }
                    if response.hovered() {
                        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
                        if scroll.abs() > 0.1 {
                            self.zoom = (self.zoom * (scroll * 0.0018).exp()).clamp(0.25, 8.0);
                        }
                    }
                    let base = fit_size(texture.size_vec2(), rect.size());
                    let size = base * self.zoom;
                    let image_rect = Rect::from_center_size(rect.center() + self.pan, size);
                    ui.painter().with_clip_rect(rect).image(
                        texture.id(),
                        image_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    if self.zoom > 1.01 {
                        ui.painter().text(
                            rect.left_bottom() + Vec2::new(8.0, -8.0),
                            egui::Align2::LEFT_BOTTOM,
                            "Drag to pan • Scroll to zoom",
                            egui::FontId::proportional(11.0),
                            Color32::from_gray(150),
                        );
                    }
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("LUMEN").size(34.0).strong().color(ACCENT));
                            ui.label(
                                RichText::new(
                                    "Drop JPEG, PNG, TIFF, WebP, or Sony ARW photos here",
                                )
                                .size(16.0)
                                .color(Color32::GRAY),
                            );
                            ui.add_space(10.0);
                            if ui.button("Choose Photos").clicked() {
                                self.import_dialog();
                            }
                        });
                    });
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
                                    "{} × {}  •  {}",
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
                ui.label(
                    "This adds a reset event to history. You can step back to recover every edit.",
                );
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
            if let Some(id) = self.workspace.project.selected
                && self.execute_and_autosave(Command::Reset { ids: vec![id] })
            {
                self.draft_id = None;
                self.sync_draft();
            }
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
        self.filmstrip(ui);
        self.inspector(ui);
        self.canvas(ui);
        self.confirmation_window(&context);
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
}

fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    changed: &mut bool,
    commit: &mut bool,
) {
    let response = ui.add(egui::Slider::new(value, range).text(label).smart_aim(false));
    *changed |= response.changed();
    *commit |= response.drag_stopped() || (response.changed() && !response.dragged());
}

fn tone_curve_editor(ui: &mut egui::Ui, curve: &mut ToneCurve, channel: usize) -> (bool, bool) {
    let desired = Vec2::new(ui.available_width(), 176.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, CANVAS);
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

fn shorten(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

#[allow(dead_code)]
fn _assert_photo_is_available(_: Photo) {}
