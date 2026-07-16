#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{collections::HashMap, path::PathBuf};

use eframe::egui::{self, Color32, RichText, Stroke, TextureHandle, TextureOptions, Vec2};
use image::DynamicImage;
use lumen_core::{
    Adjustments, Command, Photo, Workspace,
    engine::{RenderOptions, decode_photo, render_image, render_photo},
    project::is_supported_image,
};

const ACCENT: Color32 = Color32::from_rgb(232, 177, 72);
const PANEL: Color32 = Color32::from_rgb(25, 27, 31);
const CANVAS: Color32 = Color32::from_rgb(14, 15, 18);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 640.0]),
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
            style.spacing.item_spacing = Vec2::new(8.0, 8.0);
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
            status: "Drop photos anywhere, or choose Import Photos".to_owned(),
            error: false,
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
                .map(|photo| photo.adjustments)
                .unwrap_or_default();
            self.invalidate_selected();
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
            self.status = "No supported images were selected".to_owned();
            self.error = true;
            return;
        }
        if self.execute_and_autosave(Command::Import { paths }) {
            self.thumbnails.clear();
            self.draft_id = None;
            self.sync_draft();
        }
    }

    fn import_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Photos", &["jpg", "jpeg", "png", "tif", "tiff", "webp"])
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
            let rendered = render_image(source.clone(), self.draft, RenderOptions::default());
            self.preview = Some(load_texture(context, format!("preview-{id}"), rendered));
            self.preview_id = Some(id);
            self.preview_adjustments = self.draft;
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
                max_size: Some(160),
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

        let command = context.input(|input| {
            if input.modifiers.command && input.key_pressed(egui::Key::Z) {
                Some(if input.modifiers.shift {
                    Command::Redo
                } else {
                    Command::Undo
                })
            } else {
                None
            }
        });
        if let Some(command) = command
            && self.execute_and_autosave(command)
        {
            self.draft_id = None;
            self.sync_draft();
        }

        let save =
            context.input(|input| input.modifiers.command && input.key_pressed(egui::Key::S));
        if save {
            let save_as = context.input(|input| input.modifiers.shift);
            self.save_catalog(save_as);
        }

        let direction = context.input(|input| {
            if input.key_pressed(egui::Key::ArrowLeft) {
                -1
            } else if input.key_pressed(egui::Key::ArrowRight) {
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
                    if ui
                        .add_enabled(self.workspace.can_undo(), egui::Button::new("Undo"))
                        .clicked()
                        && self.execute_and_autosave(Command::Undo)
                    {
                        self.draft_id = None;
                        self.sync_draft();
                    }
                    if ui
                        .add_enabled(self.workspace.can_redo(), egui::Button::new("Redo"))
                        .clicked()
                        && self.execute_and_autosave(Command::Redo)
                    {
                        self.draft_id = None;
                        self.sync_draft();
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

    fn inspector(&mut self, root: &mut egui::Ui) {
        egui::Panel::right("inspector")
            .resizable(false)
            .exact_size(292.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(root, |ui| {
                ui.heading("Develop");
                ui.add_space(4.0);
                let Some(id) = self.workspace.project.selected else {
                    ui.label(RichText::new("Select a photo to edit").color(Color32::GRAY));
                    return;
                };

                let mut draft = self.draft;
                let mut changed = false;
                let mut commit = false;
                adjustment_slider(
                    ui,
                    "Exposure",
                    &mut draft.exposure,
                    -5.0..=5.0,
                    &mut changed,
                    &mut commit,
                );
                ui.separator();
                adjustment_slider(
                    ui,
                    "Temperature",
                    &mut draft.temperature,
                    -100.0..=100.0,
                    &mut changed,
                    &mut commit,
                );
                adjustment_slider(
                    ui,
                    "Tint",
                    &mut draft.tint,
                    -100.0..=100.0,
                    &mut changed,
                    &mut commit,
                );
                ui.separator();
                adjustment_slider(
                    ui,
                    "Contrast",
                    &mut draft.contrast,
                    -100.0..=100.0,
                    &mut changed,
                    &mut commit,
                );
                adjustment_slider(
                    ui,
                    "Highlights",
                    &mut draft.highlights,
                    -100.0..=100.0,
                    &mut changed,
                    &mut commit,
                );
                adjustment_slider(
                    ui,
                    "Shadows",
                    &mut draft.shadows,
                    -100.0..=100.0,
                    &mut changed,
                    &mut commit,
                );
                adjustment_slider(
                    ui,
                    "Whites",
                    &mut draft.whites,
                    -100.0..=100.0,
                    &mut changed,
                    &mut commit,
                );
                adjustment_slider(
                    ui,
                    "Blacks",
                    &mut draft.blacks,
                    -100.0..=100.0,
                    &mut changed,
                    &mut commit,
                );
                ui.separator();
                adjustment_slider(
                    ui,
                    "Clarity",
                    &mut draft.clarity,
                    -100.0..=100.0,
                    &mut changed,
                    &mut commit,
                );
                adjustment_slider(
                    ui,
                    "Vibrance",
                    &mut draft.vibrance,
                    -100.0..=100.0,
                    &mut changed,
                    &mut commit,
                );
                adjustment_slider(
                    ui,
                    "Saturation",
                    &mut draft.saturation,
                    -100.0..=100.0,
                    &mut changed,
                    &mut commit,
                );
                adjustment_slider(
                    ui,
                    "Vignette",
                    &mut draft.vignette,
                    -100.0..=100.0,
                    &mut changed,
                    &mut commit,
                );

                if changed {
                    self.draft = draft.sanitized();
                    self.preview = None;
                }
                if commit
                    && self.execute_and_autosave(Command::SetAdjustments {
                        id,
                        adjustments: self.draft,
                    })
                {
                    self.thumbnails.remove(&id);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("↺ 90°").clicked()
                        && self.execute_and_autosave(Command::Rotate {
                            id,
                            clockwise: false,
                        })
                    {
                        self.draft_id = None;
                        self.sync_draft();
                    }
                    if ui.button("90° ↻").clicked()
                        && self.execute_and_autosave(Command::Rotate {
                            id,
                            clockwise: true,
                        })
                    {
                        self.draft_id = None;
                        self.sync_draft();
                    }
                    if ui.button("Flip ↔").clicked()
                        && self.execute_and_autosave(Command::FlipHorizontal { id })
                    {
                        self.draft_id = None;
                        self.sync_draft();
                    }
                });
                ui.add_space(8.0);
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
                    if ui.button("Reset").clicked()
                        && self.execute_and_autosave(Command::Reset { ids: vec![id] })
                    {
                        self.draft_id = None;
                        self.sync_draft();
                    }
                });
            });
    }

    fn filmstrip(&mut self, root: &mut egui::Ui) {
        let context = root.ctx().clone();
        egui::Panel::bottom("filmstrip")
            .resizable(true)
            .default_size(132.0)
            .size_range(96.0..=220.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(root, |ui| {
                let photos: Vec<(u64, String)> = self
                    .workspace
                    .project
                    .photos
                    .iter()
                    .map(|photo| (photo.id, photo.name.clone()))
                    .collect();
                if photos.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("Your photos will appear here").color(Color32::GRAY),
                        );
                    });
                    return;
                }
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (id, name) in photos {
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
                                .inner_margin(5);
                            frame.show(ui, |ui| {
                                ui.vertical(|ui| {
                                    if let Some(texture) = self.thumbnails.get(&id) {
                                        let clicked = ui
                                            .add(
                                                egui::Button::image(
                                                    egui::Image::new(texture)
                                                        .fit_to_exact_size(Vec2::new(112.0, 74.0)),
                                                )
                                                .frame(false),
                                            )
                                            .clicked();
                                        if clicked {
                                            self.select(id);
                                        }
                                    }
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(shorten(&name, 17)).size(11.0),
                                        )
                                        .truncate(),
                                    );
                                });
                            });
                        }
                    });
                });
            });
    }

    fn canvas(&mut self, root: &mut egui::Ui) {
        let context = root.ctx().clone();
        self.ensure_preview(&context);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(CANVAS).inner_margin(20))
            .show(root, |ui| {
                if let Some(texture) = &self.preview {
                    let available = ui.available_size();
                    ui.centered_and_justified(|ui| {
                        ui.add(
                            egui::Image::new(texture)
                                .fit_to_exact_size(fit_size(texture.size_vec2(), available)),
                        );
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("LUMEN").size(34.0).strong().color(ACCENT));
                            ui.label(
                                RichText::new("Drop JPEG, PNG, TIFF, or WebP photos here")
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
                        ui.label(
                            RichText::new(format!(
                                "{} photo{}",
                                self.workspace.project.photos.len(),
                                if self.workspace.project.photos.len() == 1 {
                                    ""
                                } else {
                                    "s"
                                }
                            ))
                            .size(12.0)
                            .color(Color32::GRAY),
                        );
                    });
                });
            });
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

fn adjustment_slider(
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
    let scale = (available.x / image.x).min(available.y / image.y).min(1.0);
    image * scale
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
