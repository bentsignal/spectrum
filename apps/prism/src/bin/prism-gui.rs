#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
};

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, TextureHandle,
    TextureOptions, Vec2,
};
use lumen_core::AdjustmentPatch;
use prism_core::{
    BlendMode, Command, Document, Layer, LayerKind, LayerMask, Transform, Workspace,
    export_document,
};

#[path = "prism_gui/canvas.rs"]
mod canvas;
#[path = "prism_gui/chrome.rs"]
mod chrome;
#[path = "prism_gui/inspector.rs"]
mod inspector;
#[path = "prism_gui/layers.rs"]
mod layers;
#[path = "prism_gui/renderer.rs"]
mod renderer;
use renderer::*;

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
    action: DragAction,
    bounds: Option<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragAction {
    Move,
    Resize(ResizeHandle),
    Draw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResizeHandle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CanvasInvalidation {
    None,
    Layer(u64),
    Structure,
    All,
}

fn canvas_invalidation(command: &Command) -> CanvasInvalidation {
    match command {
        Command::UpdateText { id, .. }
        | Command::UpdateRectangle { id, .. }
        | Command::AdjustLayer { id, .. }
        | Command::ResetLayerAdjustments { id } => CanvasInvalidation::Layer(*id),
        Command::AddRaster { .. }
        | Command::AddText { .. }
        | Command::AddRectangle { .. }
        | Command::DuplicateLayer { .. }
        | Command::Undo
        | Command::Redo => CanvasInvalidation::All,
        Command::RemoveLayer { .. } => CanvasInvalidation::Structure,
        _ => CanvasInvalidation::None,
    }
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
    tab_ids: Vec<u64>,
    active_tab_id: u64,
    next_tab_id: u64,
    inactive_workspaces: HashMap<u64, Workspace>,
    layer_visuals: HashMap<u64, LayerVisualEntry>,
    layer_visual_dirty: HashSet<u64>,
    layer_render_pending: HashMap<u64, LayerRenderRequest>,
    layer_render_in_flight: bool,
    layer_render_request_sender: Sender<LayerRenderRequest>,
    layer_render_receiver: Receiver<LayerRenderResult>,
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
    layer_drag: Option<u64>,
    layer_drop_index: Option<usize>,
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
        let (layer_render_request_sender, layer_render_request_receiver) = mpsc::channel();
        let (layer_render_result_sender, layer_render_receiver) = mpsc::channel();
        spawn_layer_render_worker(
            layer_render_request_receiver,
            layer_render_result_sender,
            creation.egui_ctx.clone(),
        );
        let mut app = Self {
            workspace: Workspace::default(),
            tab_ids: vec![1],
            active_tab_id: 1,
            next_tab_id: 2,
            inactive_workspaces: HashMap::new(),
            layer_visuals: HashMap::new(),
            layer_visual_dirty: HashSet::new(),
            layer_render_pending: HashMap::new(),
            layer_render_in_flight: false,
            layer_render_request_sender,
            layer_render_receiver,
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
            layer_drag: None,
            layer_drop_index: None,
        };
        if let Some(path) = initial_project {
            app.open_path(path);
        }
        app
    }

    fn execute(&mut self, command: Command) -> bool {
        let invalidation = canvas_invalidation(&command);
        match self.workspace.execute(command) {
            Ok(output) => {
                self.apply_canvas_invalidation(invalidation);
                self.status = output.message;
                self.status_error = false;
                true
            }
            Err(error) => {
                self.status = format!("{error:#}");
                self.status_error = true;
                false
            }
        }
    }

    fn preview_command(&mut self, command: Command) -> bool {
        let invalidation = canvas_invalidation(&command);
        match self.workspace.preview(command) {
            Ok(_) => {
                self.apply_canvas_invalidation(invalidation);
                true
            }
            Err(error) => {
                self.status = format!("{error:#}");
                self.status_error = true;
                false
            }
        }
    }

    fn apply_canvas_invalidation(&mut self, invalidation: CanvasInvalidation) {
        match invalidation {
            CanvasInvalidation::None => {}
            CanvasInvalidation::Layer(id) => {
                self.layer_visual_dirty.insert(id);
            }
            CanvasInvalidation::Structure => {
                let active: HashSet<_> = self
                    .workspace
                    .document
                    .layers
                    .iter()
                    .map(|layer| layer.id)
                    .collect();
                self.layer_visuals.retain(|id, _| active.contains(id));
                self.layer_render_pending
                    .retain(|id, _| active.contains(id));
                self.layer_visual_dirty.retain(|id| active.contains(id));
            }
            CanvasInvalidation::All => {
                self.layer_visual_dirty
                    .extend(self.workspace.document.layers.iter().map(|layer| layer.id));
            }
        }
    }

    fn finish_interaction(&mut self) {
        if self.workspace.commit_interaction() {
            self.status = "Finished interaction".into();
            self.status_error = false;
        }
    }

    fn widget_command(&mut self, response: &egui::Response, command: Command) {
        if response.drag_started() {
            self.workspace.begin_interaction();
        }
        if response.changed() {
            if self.workspace.interaction_active() {
                self.preview_command(command);
            } else {
                self.execute(command);
            }
        }
        if response.drag_stopped() {
            self.finish_interaction();
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
                self.add_workspace_tab(workspace);
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
        self.add_workspace_tab(Workspace::new(
            Document::new(draft.name, draft.width, draft.height),
            None,
        ));
        self.status = "Created a new Prism document".into();
        self.status_error = false;
    }

    fn add_workspace_tab(&mut self, workspace: Workspace) {
        self.inactive_workspaces.insert(
            self.active_tab_id,
            std::mem::replace(&mut self.workspace, workspace),
        );
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tab_ids.push(id);
        self.active_tab_id = id;
        self.reset_canvas_cache();
        self.layer_thumbnails.clear();
        self.fit_requested = true;
        self.pan = Vec2::ZERO;
    }

    fn activate_tab(&mut self, id: u64) {
        if id == self.active_tab_id {
            return;
        }
        let Some(workspace) = self.inactive_workspaces.remove(&id) else {
            return;
        };
        let previous = std::mem::replace(&mut self.workspace, workspace);
        self.inactive_workspaces
            .insert(self.active_tab_id, previous);
        self.active_tab_id = id;
        self.reset_canvas_cache();
        self.layer_thumbnails.clear();
        self.fit_requested = true;
        self.pan = Vec2::ZERO;
        self.drag = None;
    }

    fn close_tab(&mut self, id: u64) {
        let dirty = if id == self.active_tab_id {
            self.workspace.is_dirty()
        } else {
            self.inactive_workspaces
                .get(&id)
                .is_some_and(Workspace::is_dirty)
        };
        if dirty {
            self.activate_tab(id);
            self.status = "Save this document before closing its tab".into();
            self.status_error = true;
            return;
        }
        if self.tab_ids.len() == 1 {
            return;
        }
        let Some(position) = self.tab_ids.iter().position(|tab| *tab == id) else {
            return;
        };
        self.tab_ids.remove(position);
        if id == self.active_tab_id {
            let replacement_id = self.tab_ids[position.min(self.tab_ids.len() - 1)];
            if let Some(replacement) = self.inactive_workspaces.remove(&replacement_id) {
                self.workspace = replacement;
                self.active_tab_id = replacement_id;
            }
        } else {
            self.inactive_workspaces.remove(&id);
        }
        self.reset_canvas_cache();
        self.layer_thumbnails.clear();
        self.fit_requested = true;
        self.pan = Vec2::ZERO;
        self.drag = None;
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
        let (image_width, image_height) = image::image_dimensions(&path).unwrap_or((0, 0));
        let x = (self.workspace.document.width as f32 - image_width as f32) * 0.5;
        let y = (self.workspace.document.height as f32 - image_height as f32) * 0.5;
        self.execute(Command::AddRaster {
            path,
            name: None,
            x,
            y,
        });
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

#[path = "prism_gui/view.rs"]
mod view;
use view::*;

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

    #[test]
    fn corner_resize_preserves_aspect_ratio_by_default() {
        let drag = DragState {
            start_canvas: Pos2::new(110.0, 70.0),
            current_canvas: Pos2::new(210.0, 90.0),
            layer_id: Some(1),
            transform: Transform {
                x: 10.0,
                y: 20.0,
                ..Default::default()
            },
            action: DragAction::Resize(ResizeHandle::BottomRight),
            bounds: Some(Rect::from_min_size(
                Pos2::new(10.0, 20.0),
                Vec2::new(100.0, 50.0),
            )),
        };
        let transform = drag_transform(drag, true);
        assert_eq!(transform.x, 10.0);
        assert_eq!(transform.y, 20.0);
        assert!((transform.scale_x - 2.0).abs() < 0.001);
        assert!((transform.scale_y - 2.0).abs() < 0.001);
    }

    #[test]
    fn shift_resize_allows_independent_axes() {
        let drag = DragState {
            start_canvas: Pos2::new(10.0, 20.0),
            current_canvas: Pos2::new(-40.0, 0.0),
            layer_id: Some(1),
            transform: Transform {
                x: 10.0,
                y: 20.0,
                ..Default::default()
            },
            action: DragAction::Resize(ResizeHandle::TopLeft),
            bounds: Some(Rect::from_min_size(
                Pos2::new(10.0, 20.0),
                Vec2::new(100.0, 50.0),
            )),
        };
        let transform = drag_transform(drag, false);
        assert!((transform.scale_x - 1.5).abs() < 0.001);
        assert!((transform.scale_y - 1.4).abs() < 0.001);
        assert_eq!(transform.x, -40.0);
        assert_eq!(transform.y, 0.0);
    }

    #[test]
    fn transforms_do_not_invalidate_cached_layer_pixels() {
        let mut layer = Layer::default();
        let before = LayerVisualKey::new(&layer);
        layer.transform = Transform {
            x: 480.0,
            y: 270.0,
            scale_x: 2.0,
            scale_y: 1.5,
            rotation: 18.0,
        };
        assert_eq!(before, LayerVisualKey::new(&layer));
    }

    #[test]
    fn command_invalidation_keeps_transform_and_appearance_on_the_gpu() {
        assert_eq!(
            canvas_invalidation(&Command::SetTransform {
                id: 7,
                transform: Transform::default(),
            }),
            CanvasInvalidation::None
        );
        assert_eq!(
            canvas_invalidation(&Command::SetOpacity {
                id: 7,
                opacity: 0.5,
            }),
            CanvasInvalidation::None
        );
        assert_eq!(
            canvas_invalidation(&Command::AdjustLayer {
                id: 7,
                patch: AdjustmentPatch {
                    exposure: Some(1.0),
                    ..Default::default()
                },
            }),
            CanvasInvalidation::Layer(7)
        );
        assert_eq!(canvas_invalidation(&Command::Undo), CanvasInvalidation::All);
    }
}
