#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, TextureHandle,
    TextureOptions, Vec2,
};
use lumen_core::AdjustmentPatch;
use prism_core::{
    BlendMode, Command, Document, Layer, LayerKind, LayerMask, Transform, Workspace,
    export_document, render_document,
};

const INK: Color32 = Color32::from_rgb(14, 16, 20);
const PANEL: Color32 = Color32::from_rgb(25, 28, 34);
const SURFACE: Color32 = Color32::from_rgb(34, 38, 46);
const RAISED: Color32 = Color32::from_rgb(45, 50, 60);
const BORDER: Color32 = Color32::from_rgb(62, 68, 80);
const TEXT: Color32 = Color32::from_rgb(226, 230, 238);
const MUTED: Color32 = Color32::from_rgb(145, 153, 169);
const ACCENT: Color32 = Color32::from_rgb(93, 216, 199);
const ACCENT_WARM: Color32 = Color32::from_rgb(247, 178, 102);
const DANGER: Color32 = Color32::from_rgb(242, 115, 121);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tool {
    #[default]
    Move,
    Crop,
    Text,
    Rectangle,
    Mask,
}

impl Tool {
    const ALL: [(Self, &'static str, &'static str); 5] = [
        (Self::Move, "V", "Select / move"),
        (Self::Crop, "C", "Crop canvas"),
        (Self::Text, "T", "Text"),
        (Self::Rectangle, "R", "Rectangle"),
        (Self::Mask, "M", "Layer mask"),
    ];
}

#[derive(Clone, Copy, Debug)]
struct CanvasGeometry {
    viewport: Rect,
    canvas: Rect,
    pixels_per_point: f32,
}

impl CanvasGeometry {
    fn screen_to_canvas(self, position: Pos2) -> Pos2 {
        Pos2::new(
            (position.x - self.canvas.left()) / self.pixels_per_point,
            (position.y - self.canvas.top()) / self.pixels_per_point,
        )
    }

    fn canvas_to_screen(self, position: Pos2) -> Pos2 {
        self.canvas.min + position.to_vec2() * self.pixels_per_point
    }
}

#[derive(Clone, Copy, Debug)]
struct DragState {
    start_canvas: Pos2,
    current_canvas: Pos2,
    layer_id: Option<u64>,
    transform: Transform,
}

#[derive(Clone, Debug)]
struct NewDocumentDialog {
    name: String,
    width: u32,
    height: u32,
}

impl Default for NewDocumentDialog {
    fn default() -> Self {
        Self {
            name: "Untitled artwork".into(),
            width: 1920,
            height: 1080,
        }
    }
}

struct PrismApp {
    workspace: Workspace,
    preview: Option<TextureHandle>,
    preview_dirty: bool,
    preview_fast: bool,
    preview_error: Option<String>,
    layer_thumbnails: HashMap<u64, TextureHandle>,
    status: String,
    status_error: bool,
    tool: Tool,
    zoom: f32,
    pan: Vec2,
    fit_requested: bool,
    drag: Option<DragState>,
    rename_layer: Option<(u64, String)>,
    new_dialog: Option<NewDocumentDialog>,
    text_dialog: Option<(Pos2, String, f32)>,
    delete_confirmation: Option<u64>,
}

fn main() -> eframe::Result {
    let initial_project = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 940.0])
            .with_min_inner_size([980.0, 640.0]),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Prism",
        options,
        Box::new(move |creation| {
            Ok(Box::new(PrismApp::new(
                creation,
                initial_project.as_deref(),
            )))
        }),
    )
}

impl PrismApp {
    fn new(creation: &eframe::CreationContext<'_>, initial_project: Option<&Path>) -> Self {
        install_style(&creation.egui_ctx);
        let mut app = Self {
            workspace: Workspace::default(),
            preview: None,
            preview_dirty: true,
            preview_fast: false,
            preview_error: None,
            layer_thumbnails: HashMap::new(),
            status: "Ready".into(),
            status_error: false,
            tool: Tool::Move,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit_requested: true,
            drag: None,
            rename_layer: None,
            new_dialog: None,
            text_dialog: None,
            delete_confirmation: None,
        };
        if let Some(path) = initial_project {
            app.open_path(path);
        }
        app
    }

    fn execute(&mut self, command: Command) -> bool {
        let affects_render = !matches!(
            &command,
            Command::SelectLayer { .. } | Command::RenameLayer { .. } | Command::SetLocked { .. }
        );
        match self.workspace.execute(command) {
            Ok(output) => {
                self.status = output.message;
                self.status_error = false;
                self.preview_dirty |= affects_render;
                true
            }
            Err(error) => {
                self.status = format!("{error:#}");
                self.status_error = true;
                false
            }
        }
    }

    fn selected_layer(&self) -> Option<&Layer> {
        self.workspace
            .document
            .selected
            .and_then(|id| self.workspace.document.layer(id).ok())
    }

    fn open_path(&mut self, path: &Path) {
        match Workspace::open(path) {
            Ok(workspace) => {
                self.workspace = workspace;
                self.preview_dirty = true;
                self.layer_thumbnails.clear();
                self.fit_requested = true;
                self.pan = Vec2::ZERO;
                self.status = format!("Opened {}", path.display());
                self.status_error = false;
            }
            Err(error) => {
                self.status = format!("Could not open project: {error:#}");
                self.status_error = true;
            }
        }
    }

    fn new_document(&mut self, draft: NewDocumentDialog) {
        self.workspace = Workspace::new(Document::new(draft.name, draft.width, draft.height), None);
        self.preview_dirty = true;
        self.layer_thumbnails.clear();
        self.fit_requested = true;
        self.pan = Vec2::ZERO;
        self.status = "Created a new Prism document".into();
        self.status_error = false;
    }

    fn save(&mut self, save_as: bool) {
        let path = if save_as || self.workspace.project_path.is_none() {
            rfd::FileDialog::new()
                .add_filter("Prism project", &["prism"])
                .set_file_name(format!("{}.prism", self.workspace.document.name))
                .save_file()
        } else {
            None
        };
        if (save_as || self.workspace.project_path.is_none()) && path.is_none() {
            return;
        }
        match self.workspace.save(path.as_deref()) {
            Ok(path) => {
                self.status = format!("Saved {}", path.display());
                self.status_error = false;
            }
            Err(error) => {
                self.status = format!("Save failed: {error:#}");
                self.status_error = true;
            }
        }
    }

    fn export(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(format!("{}.png", self.workspace.document.name))
            .save_file()
        else {
            return;
        };
        match export_document(&self.workspace.document, &path, 92) {
            Ok(()) => {
                self.status = format!("Exported {}", path.display());
                self.status_error = false;
            }
            Err(error) => {
                self.status = format!("Export failed: {error:#}");
                self.status_error = true;
            }
        }
    }

    fn add_raster(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &["jpg", "jpeg", "png", "tif", "tiff", "webp"])
            .pick_file()
        else {
            return;
        };
        self.execute(Command::AddRaster {
            path,
            name: None,
            x: 0.0,
            y: 0.0,
        });
    }

    fn ensure_preview(&mut self, context: &egui::Context) {
        let interacting = context.input(|input| input.pointer.primary_down());
        if self.preview_fast && !interacting {
            self.preview_dirty = true;
        }
        if !self.preview_dirty {
            return;
        }
        let max_size = if interacting { 960 } else { 2048 };
        match render_document(&self.workspace.document, Some(max_size)) {
            Ok(image) => {
                let rgba = image.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.preview =
                    Some(context.load_texture("prism-composite", color, TextureOptions::LINEAR));
                self.preview_error = None;
                self.preview_fast = interacting;
            }
            Err(error) => {
                self.preview = None;
                self.preview_error = Some(format!("{error:#}"));
                self.preview_fast = false;
            }
        }
        self.preview_dirty = false;
    }

    fn top_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("prism-top")
            .exact_size(54.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(8)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(3.0);
                    ui.label(RichText::new("PRISM").size(15.0).strong().color(ACCENT));
                    ui.label(RichText::new("CANVAS STUDIO").size(9.0).color(MUTED));
                    ui.separator();
                    if ui.button("New").clicked() {
                        self.new_dialog = Some(NewDocumentDialog::default());
                    }
                    if ui.button("Open").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Prism project", &["prism", "mica"])
                            .pick_file()
                    {
                        self.open_path(&path);
                    }
                    if ui.button("Save").clicked() {
                        self.save(false);
                    }
                    if ui.button("Save As").clicked() {
                        self.save(true);
                    }
                    if ui.button(RichText::new("Export").color(ACCENT)).clicked() {
                        self.export();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(self.workspace.can_undo(), egui::Button::new("Back"))
                        .clicked()
                    {
                        self.execute(Command::Undo);
                    }
                    if ui
                        .add_enabled(self.workspace.can_redo(), egui::Button::new("Forward"))
                        .clicked()
                    {
                        self.execute(Command::Redo);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let dirty = if self.workspace.is_dirty() {
                            "  Modified"
                        } else {
                            ""
                        };
                        ui.label(
                            RichText::new(format!("{}{}", self.workspace.document.name, dirty))
                                .size(12.0)
                                .color(if self.workspace.is_dirty() {
                                    ACCENT_WARM
                                } else {
                                    MUTED
                                }),
                        );
                    });
                });
            });
    }

    fn tools(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("prism-tools")
            .exact_size(58.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(7)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("TOOLS").size(9.0).color(MUTED));
                    ui.add_space(8.0);
                    for (tool, key, hint) in Tool::ALL {
                        let selected = self.tool == tool;
                        let response = ui
                            .add_sized(
                                [40.0, 40.0],
                                egui::Button::new(RichText::new(key).size(14.0).strong())
                                    .selected(selected),
                            )
                            .on_hover_text(format!("{hint} ({key})"));
                        if response.clicked() {
                            self.tool = tool;
                            self.drag = None;
                        }
                        ui.add_space(3.0);
                    }
                    ui.separator();
                    if ui
                        .add_sized([40.0, 36.0], egui::Button::new("+"))
                        .on_hover_text("Place image")
                        .clicked()
                    {
                        self.add_raster();
                    }
                });
            });
    }

    fn right_panel(&mut self, root: &mut egui::Ui) {
        egui::Panel::right("prism-inspector")
            .default_size(300.0)
            .min_size(260.0)
            .max_size(380.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(10)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                self.layers_panel(ui);
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                self.inspector(ui);
            });
    }

    fn layers_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("LAYERS").size(10.0).strong().color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("+").on_hover_text("Place image").clicked() {
                    self.add_raster();
                }
            });
        });
        ui.add_space(6.0);
        let layers = self.workspace.document.layers.to_vec();
        egui::ScrollArea::vertical()
            .id_salt("layer-stack")
            .max_height(270.0)
            .show(ui, |ui| {
                for layer in layers.iter().rev() {
                    self.layer_row(ui, layer);
                    ui.add_space(3.0);
                }
            });
        ui.horizontal(|ui| {
            let selected = self.workspace.document.selected;
            let selected_index = selected.and_then(|id| {
                self.workspace
                    .document
                    .layers
                    .iter()
                    .position(|layer| layer.id == id)
            });
            if ui
                .add_enabled(
                    selected_index
                        .is_some_and(|index| index + 1 < self.workspace.document.layers.len()),
                    egui::Button::new("Raise"),
                )
                .clicked()
                && let (Some(id), Some(index)) = (selected, selected_index)
            {
                self.execute(Command::MoveLayer {
                    id,
                    index: index + 1,
                });
            }
            if ui
                .add_enabled(
                    selected_index.is_some_and(|index| index > 0),
                    egui::Button::new("Lower"),
                )
                .clicked()
                && let (Some(id), Some(index)) = (selected, selected_index)
            {
                self.execute(Command::MoveLayer {
                    id,
                    index: index - 1,
                });
            }
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Duplicate"))
                .clicked()
                && let Some(id) = selected
            {
                self.execute(Command::DuplicateLayer { id });
            }
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Delete"))
                .clicked()
            {
                self.delete_confirmation = selected;
            }
        });
    }

    fn layer_row(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        let selected = self.workspace.document.selected == Some(layer.id);
        let thumbnail = self.layer_thumbnail(ui.ctx(), layer);
        let frame = egui::Frame::new()
            .fill(if selected {
                Color32::from_rgb(38, 67, 67)
            } else {
                SURFACE
            })
            .stroke(Stroke::new(1.0, if selected { ACCENT } else { BORDER }))
            .corner_radius(6)
            .inner_margin(6);
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let visibility = if layer.visible { "ON" } else { "--" };
                if ui
                    .small_button(visibility)
                    .on_hover_text("Toggle visibility")
                    .clicked()
                {
                    self.execute(Command::SetVisibility {
                        id: layer.id,
                        visible: !layer.visible,
                    });
                }
                let kind = match layer.kind {
                    LayerKind::Raster { .. } => "IMG",
                    LayerKind::Text { .. } => "TXT",
                    LayerKind::Rectangle { .. } => "SHP",
                };
                if let Some(texture) = thumbnail {
                    ui.add(egui::Image::new(&texture).fit_to_exact_size(Vec2::splat(30.0)));
                } else {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                    let fill = match layer.kind {
                        LayerKind::Rectangle { color, .. } | LayerKind::Text { color, .. } => {
                            Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3])
                        }
                        LayerKind::Raster { .. } => RAISED,
                    };
                    ui.painter().rect_filled(rect, 4.0, fill);
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        kind,
                        FontId::monospace(8.0),
                        contrast_text(fill),
                    );
                }
                let response = ui.selectable_label(
                    selected,
                    RichText::new(format!("{kind}  {}", layer.name)).size(12.0),
                );
                if response.clicked() {
                    self.execute(Command::SelectLayer { id: Some(layer.id) });
                }
                if response.double_clicked() {
                    self.rename_layer = Some((layer.id, layer.name.clone()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(if layer.locked { "L" } else { "U" })
                        .clicked()
                    {
                        self.execute(Command::SetLocked {
                            id: layer.id,
                            locked: !layer.locked,
                        });
                    }
                });
            });
        });
    }

    fn layer_thumbnail(&mut self, context: &egui::Context, layer: &Layer) -> Option<TextureHandle> {
        if let Some(texture) = self.layer_thumbnails.get(&layer.id) {
            return Some(texture.clone());
        }
        let LayerKind::Raster { path, .. } = &layer.kind else {
            return None;
        };
        let image = image::open(path).ok()?.thumbnail(96, 96).to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture = context.load_texture(
            format!("prism-layer-{}", layer.id),
            color,
            TextureOptions::LINEAR,
        );
        self.layer_thumbnails.insert(layer.id, texture.clone());
        Some(texture)
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("INSPECTOR").size(10.0).strong().color(MUTED));
        ui.add_space(6.0);
        let Some(layer) = self.selected_layer().cloned() else {
            ui.add_space(18.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No layer selected").color(MUTED));
                ui.label(
                    RichText::new("Choose a layer on the canvas or in the stack.")
                        .size(11.0)
                        .color(MUTED),
                );
            });
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt("inspector-scroll")
            .show(ui, |ui| {
                ui.label(RichText::new(&layer.name).size(15.0).strong());
                ui.add_space(8.0);
                self.transform_inspector(ui, &layer);
                ui.add_space(9.0);
                self.content_inspector(ui, &layer);
                ui.add_space(9.0);
                self.appearance_inspector(ui, &layer);
                ui.add_space(9.0);
                self.adjustments_inspector(ui, &layer);
            });
    }

    fn transform_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::CollapsingHeader::new("Transform")
            .default_open(true)
            .show(ui, |ui| {
                let mut transform = layer.transform;
                let mut commit = false;
                ui.horizontal(|ui| {
                    ui.label("X");
                    commit |= ui
                        .add(egui::DragValue::new(&mut transform.x).speed(1.0))
                        .changed();
                    ui.label("Y");
                    commit |= ui
                        .add(egui::DragValue::new(&mut transform.y).speed(1.0))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("W");
                    commit |= ui
                        .add(
                            egui::DragValue::new(&mut transform.scale_x)
                                .speed(0.01)
                                .range(0.01..=100.0),
                        )
                        .changed();
                    ui.label("H");
                    commit |= ui
                        .add(
                            egui::DragValue::new(&mut transform.scale_y)
                                .speed(0.01)
                                .range(0.01..=100.0),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Angle");
                    commit |= ui
                        .add(
                            egui::DragValue::new(&mut transform.rotation)
                                .speed(0.25)
                                .suffix(" deg"),
                        )
                        .changed();
                });
                if commit {
                    self.execute(Command::SetTransform {
                        id: layer.id,
                        transform,
                    });
                }
            });
    }

    fn content_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        match &layer.kind {
            LayerKind::Text {
                text,
                font_size,
                color,
            } => {
                egui::CollapsingHeader::new("Content")
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut text = text.clone();
                        let mut font_size = *font_size;
                        let mut color = color32(*color);
                        let mut changed = false;
                        ui.label(RichText::new("Text").size(11.0).color(MUTED));
                        changed |= ui
                            .add(
                                egui::TextEdit::multiline(&mut text)
                                    .desired_rows(3)
                                    .desired_width(f32::INFINITY),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut font_size, 4.0..=1_000.0)
                                    .text("Font size")
                                    .suffix(" px"),
                            )
                            .changed();
                        ui.horizontal(|ui| {
                            ui.label("Color");
                            changed |= ui.color_edit_button_srgba(&mut color).changed();
                        });
                        if changed && !text.trim().is_empty() {
                            self.execute(Command::UpdateText {
                                id: layer.id,
                                text,
                                font_size,
                                color: rgba(color),
                            });
                        }
                    });
            }
            LayerKind::Rectangle {
                width,
                height,
                color,
                corner_radius,
            } => {
                egui::CollapsingHeader::new("Content")
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut width = *width;
                        let mut height = *height;
                        let mut color = color32(*color);
                        let mut corner_radius = *corner_radius;
                        let mut changed = false;
                        ui.horizontal(|ui| {
                            ui.label("Width");
                            changed |= ui
                                .add(egui::DragValue::new(&mut width).range(1..=32_768))
                                .changed();
                            ui.label("Height");
                            changed |= ui
                                .add(egui::DragValue::new(&mut height).range(1..=32_768))
                                .changed();
                        });
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut corner_radius, 0.0..=512.0)
                                    .text("Corner radius")
                                    .suffix(" px"),
                            )
                            .changed();
                        ui.horizontal(|ui| {
                            ui.label("Fill");
                            changed |= ui.color_edit_button_srgba(&mut color).changed();
                        });
                        if changed {
                            self.execute(Command::UpdateRectangle {
                                id: layer.id,
                                width,
                                height,
                                color: rgba(color),
                                corner_radius,
                            });
                        }
                    });
            }
            LayerKind::Raster { path, .. } => {
                egui::CollapsingHeader::new("Content")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Linked image").size(11.0).color(MUTED));
                        ui.label(
                            RichText::new(path.display().to_string())
                                .size(11.0)
                                .color(TEXT),
                        );
                    });
            }
        }
    }

    fn appearance_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::CollapsingHeader::new("Appearance")
            .default_open(true)
            .show(ui, |ui| {
                let mut opacity = layer.opacity * 100.0;
                if ui
                    .add(
                        egui::Slider::new(&mut opacity, 0.0..=100.0)
                            .text("Opacity")
                            .suffix("%"),
                    )
                    .changed()
                {
                    self.execute(Command::SetOpacity {
                        id: layer.id,
                        opacity: opacity / 100.0,
                    });
                }
                let mut blend = layer.blend_mode;
                egui::ComboBox::from_label("Blend")
                    .selected_text(blend_label(blend))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut blend, BlendMode::Normal, "Normal");
                        ui.selectable_value(&mut blend, BlendMode::Multiply, "Multiply");
                        ui.selectable_value(&mut blend, BlendMode::Screen, "Screen");
                        ui.selectable_value(&mut blend, BlendMode::Overlay, "Overlay");
                    });
                if blend != layer.blend_mode {
                    self.execute(Command::SetBlendMode {
                        id: layer.id,
                        blend_mode: blend,
                    });
                }
                let mut clipped = layer.clip_to_below;
                if ui.checkbox(&mut clipped, "Clip to layer below").changed() {
                    self.execute(Command::SetClipping {
                        id: layer.id,
                        enabled: clipped,
                    });
                }
                let mut mask = layer.mask;
                if ui
                    .checkbox(&mut mask.enabled, "Enable rectangular mask")
                    .changed()
                {
                    self.execute(Command::SetMask { id: layer.id, mask });
                }
                if mask.enabled {
                    let mut changed = false;
                    changed |= ui
                        .add(egui::Slider::new(&mut mask.x, 0.0..=0.99).text("Mask X"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut mask.y, 0.0..=0.99).text("Mask Y"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut mask.width, 0.01..=1.0).text("Mask width"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut mask.height, 0.01..=1.0).text("Mask height"))
                        .changed();
                    changed |= ui.checkbox(&mut mask.invert, "Invert mask").changed();
                    if changed {
                        self.execute(Command::SetMask { id: layer.id, mask });
                    }
                }
            });
    }

    fn adjustments_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::CollapsingHeader::new("Develop")
            .default_open(true)
            .show(ui, |ui| {
                let a = &layer.adjustments;
                self.adjustment_slider(ui, layer.id, "Exposure", a.exposure, -5.0..=5.0, |value| {
                    AdjustmentPatch {
                        exposure: Some(value),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Contrast",
                    a.contrast,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        contrast: Some(value),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Highlights",
                    a.highlights,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        highlights: Some(value),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Shadows",
                    a.shadows,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        shadows: Some(value),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Temperature",
                    a.temperature,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        temperature: Some(value),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Vibrance",
                    a.vibrance,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        vibrance: Some(value),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Saturation",
                    a.saturation,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        saturation: Some(value),
                        ..Default::default()
                    },
                );
                if ui.button("Reset layer development").clicked() {
                    self.execute(Command::ResetLayerAdjustments { id: layer.id });
                }
            });
    }

    fn adjustment_slider(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        label: &str,
        current: f32,
        range: std::ops::RangeInclusive<f32>,
        patch: impl FnOnce(f32) -> AdjustmentPatch,
    ) {
        let mut value = current;
        if ui
            .add(egui::Slider::new(&mut value, range).text(label))
            .changed()
        {
            self.execute(Command::AdjustLayer {
                id,
                patch: patch(value),
            });
        }
    }

    fn canvas(&mut self, root: &mut egui::Ui) {
        self.ensure_preview(root.ctx());
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(INK).inner_margin(20))
            .show(root, |ui| {
                let available = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(available, Sense::click_and_drag());
                let geometry = canvas_geometry(
                    available,
                    self.workspace.document.width,
                    self.workspace.document.height,
                    self.zoom,
                    self.pan,
                );
                if self.fit_requested {
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                    self.fit_requested = false;
                }
                paint_workspace(ui, geometry, self.preview.as_ref());
                if let Some(layer) = self.selected_layer() {
                    paint_layer_outline(ui, geometry, layer, Vec2::ZERO);
                }
                self.canvas_interaction(ui, &response, geometry);
                if let Some(error) = &self.preview_error {
                    ui.painter().text(
                        available.center(),
                        Align2::CENTER_CENTER,
                        error,
                        FontId::proportional(13.0),
                        DANGER,
                    );
                }
            });
    }

    fn canvas_interaction(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        geometry: CanvasGeometry,
    ) {
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 {
                self.zoom = (self.zoom * (scroll * 0.0015).exp()).clamp(0.1, 16.0);
            }
        }
        if ui.input(|input| input.pointer.middle_down()) && response.dragged() {
            self.pan += response.drag_delta();
            return;
        }
        let pointer = response.interact_pointer_pos();
        if response.drag_started()
            && let Some(pointer) = pointer
        {
            let canvas = geometry.screen_to_canvas(pointer);
            if self.tool == Tool::Move {
                let hit = self.hit_test_layer(canvas);
                if hit != self.workspace.document.selected {
                    self.execute(Command::SelectLayer { id: hit });
                }
            }
            let selected = self.selected_layer().cloned();
            self.drag = Some(DragState {
                start_canvas: canvas,
                current_canvas: canvas,
                layer_id: selected.as_ref().map(|layer| layer.id),
                transform: selected.map(|layer| layer.transform).unwrap_or_default(),
            });
        }
        if response.dragged()
            && let (Some(pointer), Some(drag)) = (pointer, self.drag.as_mut())
        {
            drag.current_canvas = geometry.screen_to_canvas(pointer);
        }
        if let Some(drag) = self.drag {
            self.paint_drag(ui, geometry, drag);
        }
        if response.drag_stopped()
            && let Some(drag) = self.drag.take()
        {
            self.finish_canvas_drag(drag);
        } else if response.clicked()
            && let Some(pointer) = pointer
        {
            self.canvas_click(geometry.screen_to_canvas(pointer));
        }
    }

    fn paint_drag(&self, ui: &egui::Ui, geometry: CanvasGeometry, drag: DragState) {
        let start = geometry.canvas_to_screen(drag.start_canvas);
        let current = geometry.canvas_to_screen(drag.current_canvas);
        match self.tool {
            Tool::Rectangle | Tool::Crop | Tool::Mask => {
                let rect = Rect::from_two_pos(start, current);
                ui.painter().rect_filled(
                    rect,
                    1.0,
                    Color32::from_rgba_unmultiplied(93, 216, 199, 30),
                );
                ui.painter().rect_stroke(
                    rect,
                    1.0,
                    Stroke::new(1.5, ACCENT),
                    egui::StrokeKind::Inside,
                );
            }
            Tool::Move => {
                let delta = current - start;
                if let Some(id) = drag.layer_id
                    && let Ok(layer) = self.workspace.document.layer(id)
                {
                    paint_layer_outline(ui, geometry, layer, delta);
                }
                ui.painter().text(
                    current,
                    Align2::LEFT_BOTTOM,
                    format!(
                        "{:+.0}, {:+.0}",
                        delta.x / geometry.pixels_per_point,
                        delta.y / geometry.pixels_per_point
                    ),
                    FontId::monospace(11.0),
                    ACCENT,
                );
            }
            Tool::Text => {}
        }
    }

    fn finish_canvas_drag(&mut self, drag: DragState) {
        let min = drag.start_canvas.min(drag.current_canvas);
        let max = drag.start_canvas.max(drag.current_canvas);
        let size = max - min;
        match self.tool {
            Tool::Move => {
                if let Some(id) = drag.layer_id {
                    let delta = drag.current_canvas - drag.start_canvas;
                    let mut transform = drag.transform;
                    transform.x += delta.x;
                    transform.y += delta.y;
                    self.execute(Command::SetTransform { id, transform });
                }
            }
            Tool::Rectangle if size.x > 2.0 && size.y > 2.0 => {
                self.execute(Command::AddRectangle {
                    name: None,
                    width: size.x.round().max(1.0) as u32,
                    height: size.y.round().max(1.0) as u32,
                    color: [93, 216, 199, 255],
                    corner_radius: 10.0,
                    x: min.x,
                    y: min.y,
                });
            }
            Tool::Crop if size.x > 2.0 && size.y > 2.0 => {
                self.execute(Command::CropCanvas {
                    x: min.x.max(0.0).round() as u32,
                    y: min.y.max(0.0).round() as u32,
                    width: size.x.round() as u32,
                    height: size.y.round() as u32,
                });
                self.fit_requested = true;
            }
            Tool::Mask if size.x > 2.0 && size.y > 2.0 => {
                if let Some(id) = drag.layer_id {
                    let width = self.workspace.document.width as f32;
                    let height = self.workspace.document.height as f32;
                    self.execute(Command::SetMask {
                        id,
                        mask: LayerMask {
                            enabled: true,
                            x: (min.x / width).clamp(0.0, 0.99),
                            y: (min.y / height).clamp(0.0, 0.99),
                            width: (size.x / width).clamp(0.001, 1.0),
                            height: (size.y / height).clamp(0.001, 1.0),
                            invert: false,
                        },
                    });
                }
            }
            _ => {}
        }
    }

    fn canvas_click(&mut self, position: Pos2) {
        match self.tool {
            Tool::Move => {
                let hit = self.hit_test_layer(position);
                if hit != self.workspace.document.selected {
                    self.execute(Command::SelectLayer { id: hit });
                }
            }
            Tool::Text => self.text_dialog = Some((position, "Text".into(), 72.0)),
            Tool::Rectangle => {
                self.execute(Command::AddRectangle {
                    name: None,
                    width: 320,
                    height: 180,
                    color: [93, 216, 199, 255],
                    corner_radius: 12.0,
                    x: position.x,
                    y: position.y,
                });
            }
            _ => {}
        }
    }

    fn hit_test_layer(&self, position: Pos2) -> Option<u64> {
        self.workspace
            .document
            .layers
            .iter()
            .rev()
            .filter(|layer| layer.visible)
            .find(|layer| layer_bounds(layer).is_some_and(|rect| rect.contains(position)))
            .map(|layer| layer.id)
    }

    fn status_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("prism-status")
            .exact_size(30.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(6)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&self.status)
                            .size(11.0)
                            .color(if self.status_error { DANGER } else { MUTED }),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Fit").clicked() {
                            self.zoom = 1.0;
                            self.pan = Vec2::ZERO;
                        }
                        ui.add(
                            egui::Slider::new(&mut self.zoom, 0.1..=8.0)
                                .logarithmic(true)
                                .show_value(false),
                        );
                        ui.label(
                            RichText::new(format!("{:.0}%", self.zoom * 100.0))
                                .monospace()
                                .size(11.0)
                                .color(MUTED),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "{} x {} px",
                                self.workspace.document.width, self.workspace.document.height
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    });
                });
            });
    }

    fn dialogs(&mut self, context: &egui::Context) {
        self.new_document_dialog(context);
        self.text_dialog(context);
        self.rename_dialog(context);
        self.delete_dialog(context);
    }

    fn new_document_dialog(&mut self, context: &egui::Context) {
        let Some(mut draft) = self.new_dialog.take() else {
            return;
        };
        let mut create = false;
        let mut keep_open = true;
        egui::Window::new("New Prism document")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut draft.name);
                ui.horizontal(|ui| {
                    ui.label("Width");
                    ui.add(egui::DragValue::new(&mut draft.width).range(1..=32_768));
                    ui.label("Height");
                    ui.add(egui::DragValue::new(&mut draft.height).range(1..=32_768));
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        keep_open = false;
                    }
                    if ui
                        .button(RichText::new("Create canvas").color(ACCENT))
                        .clicked()
                    {
                        create = true;
                        keep_open = false;
                    }
                });
            });
        if create {
            self.new_document(draft);
        } else if keep_open {
            self.new_dialog = Some(draft);
        }
    }

    fn text_dialog(&mut self, context: &egui::Context) {
        let Some((position, mut text, mut size)) = self.text_dialog.take() else {
            return;
        };
        let mut insert = false;
        let mut keep_open = true;
        egui::Window::new("Add text")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.text_edit_multiline(&mut text);
                ui.add(egui::Slider::new(&mut size, 8.0..=400.0).text("Size"));
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        keep_open = false;
                    }
                    if ui.button(RichText::new("Add text").color(ACCENT)).clicked() {
                        insert = true;
                        keep_open = false;
                    }
                });
            });
        if insert {
            self.execute(Command::AddText {
                text,
                name: None,
                font_size: size,
                color: [245, 246, 250, 255],
                x: position.x,
                y: position.y,
            });
        } else if keep_open {
            self.text_dialog = Some((position, text, size));
        }
    }

    fn rename_dialog(&mut self, context: &egui::Context) {
        let Some((id, mut name)) = self.rename_layer.take() else {
            return;
        };
        let mut save = false;
        let mut keep_open = true;
        egui::Window::new("Rename layer")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                let response = ui.text_edit_singleline(&mut name);
                response.request_focus();
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    save = true;
                    keep_open = false;
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        keep_open = false;
                    }
                    if ui.button("Rename").clicked() {
                        save = true;
                        keep_open = false;
                    }
                });
            });
        if save {
            self.execute(Command::RenameLayer { id, name });
        } else if keep_open {
            self.rename_layer = Some((id, name));
        }
    }

    fn delete_dialog(&mut self, context: &egui::Context) {
        let Some(id) = self.delete_confirmation else {
            return;
        };
        let mut delete = false;
        let mut cancel = false;
        egui::Window::new("Delete layer?")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label("This removes the layer from the Prism document.");
                ui.label(
                    RichText::new("Linked source image files are never deleted.").color(ACCENT),
                );
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(RichText::new("Delete layer").color(DANGER))
                        .clicked()
                    {
                        delete = true;
                    }
                });
            });
        if delete {
            self.delete_confirmation = None;
            self.execute(Command::RemoveLayer { id });
        }
        if cancel {
            self.delete_confirmation = None;
        }
    }

    fn keyboard(&mut self, context: &egui::Context) {
        if context.egui_wants_keyboard_input() {
            return;
        }
        context.input(|input| {
            if input.key_pressed(egui::Key::V) {
                self.tool = Tool::Move;
            }
            if input.key_pressed(egui::Key::C) {
                self.tool = Tool::Crop;
            }
            if input.key_pressed(egui::Key::T) {
                self.tool = Tool::Text;
            }
            if input.key_pressed(egui::Key::R) {
                self.tool = Tool::Rectangle;
            }
            if input.key_pressed(egui::Key::M) {
                self.tool = Tool::Mask;
            }
        });
        if context.input(|input| input.modifiers.command && input.key_pressed(egui::Key::S)) {
            self.save(false);
        }
        if context.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Z)) {
            if context.input(|input| input.modifiers.shift) {
                self.execute(Command::Redo);
            } else {
                self.execute(Command::Undo);
            }
        }
        if context.input(|input| input.key_pressed(egui::Key::Delete)) {
            self.delete_confirmation = self.workspace.document.selected;
        }
    }
}

impl eframe::App for PrismApp {
    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        let _ = frame;
        let context = root.ctx().clone();
        self.keyboard(&context);
        self.top_bar(root);
        self.status_bar(root);
        self.tools(root);
        self.right_panel(root);
        self.canvas(root);
        self.dialogs(&context);
        if self.preview_dirty {
            context.request_repaint();
        }
    }
}

fn install_style(context: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = INK;
    visuals.faint_bg_color = SURFACE;
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke.color = Color32::BLACK;
    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.inactive.bg_fill = RAISED;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(57, 63, 74);
    visuals.widgets.active.bg_fill = Color32::from_rgb(39, 91, 85);
    visuals.widgets.inactive.corner_radius = 5.into();
    visuals.widgets.hovered.corner_radius = 5.into();
    visuals.widgets.active.corner_radius = 5.into();
    visuals.override_text_color = Some(TEXT);
    context.set_visuals(visuals);
    context.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(7.0, 7.0);
        style.spacing.button_padding = Vec2::new(9.0, 6.0);
        style.interaction.selectable_labels = false;
    });
}

fn canvas_geometry(
    viewport: Rect,
    width: u32,
    height: u32,
    zoom: f32,
    pan: Vec2,
) -> CanvasGeometry {
    let image = Vec2::new(width.max(1) as f32, height.max(1) as f32);
    let fit = (viewport.width() / image.x)
        .min(viewport.height() / image.y)
        .min(1.0);
    let pixels_per_point = fit * zoom;
    let canvas = Rect::from_center_size(viewport.center() + pan, image * pixels_per_point);
    CanvasGeometry {
        viewport,
        canvas,
        pixels_per_point,
    }
}

fn paint_workspace(ui: &egui::Ui, geometry: CanvasGeometry, texture: Option<&TextureHandle>) {
    ui.painter().rect_filled(geometry.viewport, 0.0, INK);
    let clipped = geometry.canvas.intersect(geometry.viewport);
    let checker = 14.0;
    let cols = (clipped.width() / checker).ceil() as i32 + 1;
    let rows = (clipped.height() / checker).ceil() as i32 + 1;
    for row in 0..rows {
        for col in 0..cols {
            let min = clipped.min + Vec2::new(col as f32 * checker, row as f32 * checker);
            let cell = Rect::from_min_size(min, Vec2::splat(checker)).intersect(clipped);
            let color = if (row + col) % 2 == 0 {
                Color32::from_rgb(64, 67, 73)
            } else {
                Color32::from_rgb(49, 52, 58)
            };
            ui.painter().rect_filled(cell, 0.0, color);
        }
    }
    if let Some(texture) = texture {
        ui.painter().with_clip_rect(geometry.viewport).image(
            texture.id(),
            geometry.canvas,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    ui.painter().rect_stroke(
        geometry.canvas,
        1.0,
        Stroke::new(1.0, Color32::from_gray(90)),
        egui::StrokeKind::Outside,
    );
}

fn layer_bounds(layer: &Layer) -> Option<Rect> {
    let size = match &layer.kind {
        LayerKind::Raster { path, .. } => {
            let (width, height) = image::image_dimensions(path).ok()?;
            Vec2::new(width as f32, height as f32)
        }
        LayerKind::Text {
            text, font_size, ..
        } => {
            let longest = text.lines().map(str::len).max().unwrap_or(1) as f32;
            let lines = text.lines().count().max(1) as f32;
            Vec2::new(longest * font_size * 0.55, lines * font_size * 1.25)
        }
        LayerKind::Rectangle { width, height, .. } => Vec2::new(*width as f32, *height as f32),
    };
    let size = Vec2::new(
        size.x * layer.transform.scale_x,
        size.y * layer.transform.scale_y,
    );
    Some(Rect::from_min_size(
        Pos2::new(layer.transform.x, layer.transform.y),
        size,
    ))
}

fn paint_layer_outline(ui: &egui::Ui, geometry: CanvasGeometry, layer: &Layer, offset: Vec2) {
    let Some(bounds) = layer_bounds(layer) else {
        return;
    };
    let rect = Rect::from_min_max(
        geometry.canvas_to_screen(bounds.min) + offset,
        geometry.canvas_to_screen(bounds.max) + offset,
    );
    ui.painter().with_clip_rect(geometry.viewport).rect_stroke(
        rect,
        1.0,
        Stroke::new(1.5, ACCENT),
        egui::StrokeKind::Outside,
    );
    for corner in [
        rect.left_top(),
        rect.right_top(),
        rect.left_bottom(),
        rect.right_bottom(),
    ] {
        ui.painter().rect_filled(
            Rect::from_center_size(corner, Vec2::splat(7.0)),
            1.0,
            ACCENT,
        );
    }
}

fn blend_label(mode: BlendMode) -> &'static str {
    match mode {
        BlendMode::Normal => "Normal",
        BlendMode::Multiply => "Multiply",
        BlendMode::Screen => "Screen",
        BlendMode::Overlay => "Overlay",
    }
}

fn contrast_text(background: Color32) -> Color32 {
    let luma = background.r() as u16 * 3 + background.g() as u16 * 6 + background.b() as u16;
    if luma > 1_400 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

fn color32(value: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(value[0], value[1], value[2], value[3])
}

fn rgba(value: Color32) -> [u8; 4] {
    [value.r(), value.g(), value.b(), value.a()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_canvas_preserves_aspect_ratio() {
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0));
        let geometry = canvas_geometry(viewport, 1920, 1080, 1.0, Vec2::ZERO);
        assert!((geometry.canvas.width() / geometry.canvas.height() - 16.0 / 9.0).abs() < 0.001);
        assert!(geometry.canvas.width() <= viewport.width());
        assert!(geometry.canvas.height() <= viewport.height());
    }

    #[test]
    fn canvas_coordinate_round_trip_is_stable() {
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let geometry = canvas_geometry(viewport, 400, 300, 1.5, Vec2::new(13.0, -7.0));
        let point = Pos2::new(123.0, 87.0);
        let round_trip = geometry.screen_to_canvas(geometry.canvas_to_screen(point));
        assert!((round_trip.x - point.x).abs() < 0.001);
        assert!((round_trip.y - point.y).abs() < 0.001);
    }
}
