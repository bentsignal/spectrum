#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
};

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, TextureHandle,
    TextureOptions, Vec2,
};
use prism_core::{
    BlendMode, Command, Document, Layer, LayerKind, LayerMask, ShapeStroke, Transform, Workspace,
    export_document,
};
use spectrum_imaging::AdjustmentPatch;

#[path = "prism_gui/canvas.rs"]
mod canvas;
#[path = "prism_gui/chrome.rs"]
mod chrome;
#[path = "prism_gui/compositor.rs"]
mod compositor;
#[path = "prism_gui/dialogs.rs"]
mod dialogs;
#[path = "prism_gui/history.rs"]
mod history;
#[path = "prism_gui/inspector.rs"]
mod inspector;
#[path = "prism_gui/layers.rs"]
mod layers;
#[cfg(target_os = "macos")]
#[path = "prism_gui/macos.rs"]
mod macos;
#[path = "prism_gui/project_lifecycle.rs"]
mod project_lifecycle;
#[path = "prism_gui/renderer.rs"]
mod renderer;
#[path = "prism_gui/shortcuts.rs"]
mod shortcuts;
use compositor::*;
use history::HistoryViewState;
use project_lifecycle::MoveProjectDialog;
use renderer::*;

const INK: Color32 = Color32::from_rgb(14, 16, 20);
const PANEL: Color32 = Color32::from_rgb(25, 28, 34);
const SURFACE: Color32 = Color32::from_rgb(34, 38, 46);
const RAISED: Color32 = Color32::from_rgb(45, 50, 60);
const HOVER_SURFACE: Color32 = Color32::from_rgb(57, 63, 74);
const ACTIVE_SURFACE: Color32 = Color32::from_rgb(39, 91, 85);
const SELECTED_SURFACE: Color32 = Color32::from_rgb(36, 58, 60);
const BORDER: Color32 = Color32::from_rgb(62, 68, 80);
const TEXT: Color32 = Color32::from_rgb(226, 230, 238);
const MUTED: Color32 = Color32::from_rgb(145, 153, 169);
const ACCENT: Color32 = Color32::from_rgb(93, 216, 199);
const ACCENT_WARM: Color32 = Color32::from_rgb(247, 178, 102);
const DANGER: Color32 = Color32::from_rgb(242, 115, 121);
const CANVAS_EDGE: Color32 = Color32::from_gray(90);
const CHECKER_LIGHT: Color32 = Color32::from_rgb(64, 67, 73);
const CHECKER_DARK: Color32 = Color32::from_rgb(49, 52, 58);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tool {
    #[default]
    Move,
    Crop,
    Text,
    Shape,
    Mask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolActivation {
    ImmediateDialog,
    ChoiceDialog,
    CanvasGesture,
}

impl Tool {
    const ALL: [(Self, &'static str, &'static str); 5] = [
        (Self::Move, "V", "Select / move"),
        (Self::Crop, "C", "Crop canvas"),
        (Self::Text, "T", "Text"),
        (Self::Shape, "S", "Shape"),
        (Self::Mask, "M", "Layer mask"),
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Crop => "Crop canvas",
            Self::Text => "Add text",
            Self::Shape => "Shape",
            Self::Mask => "Draw mask",
        }
    }

    fn shortcut(self) -> &'static str {
        Self::ALL
            .iter()
            .find_map(|(tool, key, _)| (*tool == self).then_some(*key))
            .unwrap_or_default()
    }

    fn description(self) -> &'static str {
        match self {
            Self::Move => "Select on the canvas, drag to move, or pull a corner to resize.",
            Self::Crop => "Draw the new canvas boundary.",
            Self::Text => "Type the text now, then move it into place.",
            Self::Shape => "Choose a rectangle, ellipse, or another shape to draw.",
            Self::Mask => "Draw the visible region of the focused element.",
        }
    }

    fn activation(self) -> ToolActivation {
        match self {
            Self::Text => ToolActivation::ImmediateDialog,
            Self::Shape => ToolActivation::ChoiceDialog,
            _ => ToolActivation::CanvasGesture,
        }
    }

    fn matches(self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        query.is_empty()
            || self.label().to_ascii_lowercase().contains(&query)
            || self.description().to_ascii_lowercase().contains(&query)
    }
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
        | Command::UpdateEllipse { id, .. }
        | Command::SetShapeStroke { id, .. }
        | Command::RasterizeShape { id, .. }
        | Command::AdjustLayer { id, .. }
        | Command::ResetLayerAdjustments { id } => CanvasInvalidation::Layer(*id),
        Command::AddRaster { .. }
        | Command::AddText { .. }
        | Command::AddRectangle { .. }
        | Command::AddEllipse { .. }
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

#[derive(Clone, Copy, Debug)]
enum TextDialogTarget {
    New { position: Pos2 },
    Existing { id: u64 },
}

#[derive(Clone, Debug)]
struct TextDialogDraft {
    target: TextDialogTarget,
    text: String,
    font_size: f32,
    color: [u8; 4],
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
    composite_preview: CompositePreview,
    preview_error: Option<String>,
    layer_thumbnails: HashMap<u64, TextureHandle>,
    status: String,
    status_error: bool,
    tool: Tool,
    shape_kind: chrome::ShapeKind,
    tool_palette: Option<chrome::PaletteState>,
    shape_palette: Option<chrome::PaletteState>,
    inspector_section: inspector::InspectorSection,
    composition_query: String,
    composition_search_focus: bool,
    composition_result_index: usize,
    zoom: f32,
    pan: Vec2,
    fit_requested: bool,
    drag: Option<DragState>,
    rename_layer: Option<(u64, String)>,
    new_dialog: Option<NewDocumentDialog>,
    text_dialog: Option<TextDialogDraft>,
    delete_confirmation: Option<u64>,
    layer_drag: Option<u64>,
    layer_drop_index: Option<usize>,
    move_project_dialog: Option<MoveProjectDialog>,
    history: HistoryViewState,
    open_document_receiver: Receiver<PathBuf>,
    pending_open_documents: VecDeque<(std::time::Instant, PathBuf)>,
    startup_project_ready_at: std::time::Instant,
    collaboration_poll_at: std::time::Instant,
    workspace_initialized: bool,
}

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 940.0])
            .with_min_inner_size([980.0, 640.0]),
        centered: true,
        ..Default::default()
    }
}

#[cfg(not(target_os = "macos"))]
fn main() -> eframe::Result {
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
fn main() -> eframe::Result {
    let initial_project = std::env::args_os().nth(1).map(PathBuf::from);
    let (open_document_sender, open_document_receiver) = mpsc::channel();
    let event_loop =
        winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event().build()?;
    macos::install_open_document_handler(open_document_sender);
    let mut application = eframe::create_native(
        "Prism",
        native_options(),
        Box::new(move |creation| {
            Ok(Box::new(PrismApp::new(
                creation,
                initial_project.as_deref(),
                open_document_receiver,
            )))
        }),
        &event_loop,
    );
    event_loop.run_app(&mut application)?;
    Ok(())
}

impl PrismApp {
    fn new(
        creation: &eframe::CreationContext<'_>,
        initial_project: Option<&Path>,
        open_document_receiver: Receiver<PathBuf>,
    ) -> Self {
        install_style(&creation.egui_ctx);
        let (layer_render_request_sender, layer_render_request_receiver) = mpsc::channel();
        let (layer_render_result_sender, layer_render_receiver) = mpsc::channel();
        spawn_layer_render_worker(
            layer_render_request_receiver,
            layer_render_result_sender,
            creation.egui_ctx.clone(),
        );
        let initial_workspace =
            initial_project.and_then(|path| project_lifecycle::open_local_workspace(path).ok());
        let workspace_initialized = initial_workspace.is_some();
        let mut app = Self {
            workspace: initial_workspace.unwrap_or_default(),
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
            composite_preview: CompositePreview::new(creation.egui_ctx.clone()),
            preview_error: None,
            layer_thumbnails: HashMap::new(),
            status: "Ready".into(),
            status_error: false,
            tool: Tool::Move,
            shape_kind: chrome::ShapeKind::Rectangle,
            tool_palette: None,
            shape_palette: None,
            inspector_section: inspector::InspectorSection::default(),
            composition_query: String::new(),
            composition_search_focus: false,
            composition_result_index: 0,
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
            move_project_dialog: None,
            history: HistoryViewState::new(creation.egui_ctx.clone()),
            open_document_receiver,
            pending_open_documents: VecDeque::new(),
            startup_project_ready_at: std::time::Instant::now()
                + std::time::Duration::from_millis(250),
            collaboration_poll_at: std::time::Instant::now(),
            workspace_initialized,
        };
        if let Some(path) = initial_project {
            if workspace_initialized {
                app.status = format!("Opened {}", path.display());
            } else {
                app.status = format!("Could not open project {}", path.display());
                app.status_error = true;
            }
        }
        app
    }

    fn execute(&mut self, command: Command) -> bool {
        let invalidation = canvas_invalidation(&command);
        match self.workspace.execute(command) {
            Ok(output) => {
                self.apply_canvas_invalidation(invalidation);
                if let Some(error) = self.workspace.pending_publish_error() {
                    self.status = format!(
                        "Edit is safe in Prism recovery storage, but the project file could not update: {error}"
                    );
                    self.status_error = true;
                } else {
                    self.status = output.message;
                    self.status_error = false;
                }
                self.history.mark_stale();
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
        match self.workspace.commit_interaction() {
            Ok(true) => {
                if let Some(error) = self.workspace.pending_publish_error() {
                    self.status = format!(
                        "Edit is safe in Prism recovery storage, but the project file could not update: {error}"
                    );
                    self.status_error = true;
                } else {
                    self.status = "Finished interaction".into();
                    self.status_error = false;
                }
            }
            Ok(false) => {}
            Err(error) => {
                self.status = format!("Could not record interaction: {error:#}");
                self.status_error = true;
            }
        }
    }

    fn widget_command(&mut self, response: &egui::Response, command: Command) {
        self.widget_command_if(response, Some(command));
    }

    fn widget_command_if(&mut self, response: &egui::Response, command: Option<Command>) {
        if response.drag_started() || response.gained_focus() {
            self.workspace.begin_interaction();
        }
        if response.changed()
            && let Some(command) = command
        {
            if self.workspace.interaction_active() {
                self.preview_command(command);
            } else {
                self.execute(command);
            }
        }
        if response.drag_stopped() || response.lost_focus() {
            self.finish_interaction();
        }
    }

    fn selected_layer(&self) -> Option<&Layer> {
        self.workspace
            .document
            .selected
            .and_then(|id| self.workspace.document.layer(id).ok())
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
        self.history.workspace_changed();
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
        self.history.workspace_changed();
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
            self.status = "This legacy document must be converted before its tab can close".into();
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
        self.history.workspace_changed();
        self.reset_canvas_cache();
        self.layer_thumbnails.clear();
        self.fit_requested = true;
        self.pan = Vec2::ZERO;
        self.drag = None;
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
}

impl eframe::App for PrismApp {
    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        let _ = frame;
        #[cfg(target_os = "macos")]
        spectrum_history_ui::reserve_history_shortcut();
        let context = root.ctx().clone();
        self.receive_open_documents(&context);
        self.sync_agent_collaborations(&context);
        self.keyboard(&context);
        self.top_bar(root);
        if self.history.visible {
            self.history_view(root);
        } else {
            self.workbench_bar(root);
            self.status_bar(root);
            self.right_panel(root);
            self.canvas(root);
        }
        self.dialogs(&context);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.finish_interaction();
        let _ = self.workspace.checkpoint();
        for workspace in self.inactive_workspaces.values() {
            let _ = workspace.checkpoint();
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
    visuals.widgets.hovered.bg_fill = HOVER_SURFACE;
    visuals.widgets.active.bg_fill = ACTIVE_SURFACE;
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
            current_canvas: Pos2::new(210.0, 120.0),
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
    fn proportional_resize_changes_smoothly_across_axis_dominance() {
        let make_drag = |current_canvas| DragState {
            start_canvas: Pos2::new(110.0, 70.0),
            current_canvas,
            layer_id: Some(1),
            transform: Transform {
                x: 10.0,
                y: 20.0,
                ..Default::default()
            },
            action: DragAction::Resize(ResizeHandle::BottomRight),
            bounds: Some(Rect::from_min_size(
                Pos2::new(10.0, 20.0),
                Vec2::new(500.0, 100.0),
            )),
        };
        let before = drag_transform(make_drag(Pos2::new(560.0, 129.0)), true);
        let after = drag_transform(make_drag(Pos2::new(560.0, 131.0)), true);
        assert!(after.scale_x > before.scale_x);
        assert!((after.scale_x - before.scale_x - 0.01).abs() < 0.001);
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
    fn shape_moves_and_rotations_reuse_geometry_at_the_same_scale() {
        let mut layer = Layer::default();
        let before = LayerVisualKey::new(&layer, 1.0);
        layer.transform = Transform {
            x: 480.0,
            y: 270.0,
            rotation: 18.0,
            ..Default::default()
        };
        assert_eq!(before, LayerVisualKey::new(&layer, 1.0));
    }

    #[test]
    fn scaled_shapes_request_resolution_from_geometry() {
        let mut layer = Layer::default();
        let before = LayerVisualKey::new(&layer, 1.0);
        layer.transform.scale_x = 2.1;
        layer.transform.scale_y = 5.0;
        let scaled = LayerVisualKey::new(&layer, 1.0);
        assert_ne!(before, scaled);
        assert_eq!(scaled.shape_raster_scale, [4, 8]);
    }

    #[test]
    fn scaled_text_requests_a_higher_resolution_preview() {
        let mut layer = Layer {
            kind: LayerKind::Text {
                text: "testing".into(),
                font_size: 72.0,
                color: [255, 255, 255, 255],
            },
            ..Default::default()
        };
        let before = LayerVisualKey::new(&layer, 1.0);
        layer.transform.scale_x = 3.0;
        layer.transform.scale_y = 3.0;
        let scaled = LayerVisualKey::new(&layer, 1.0);
        assert_ne!(before, scaled);
        assert_eq!(scaled.text_raster_scale, 4);
    }

    #[test]
    fn active_text_resize_reuses_the_cached_raster_scale() {
        let mut layer = Layer {
            kind: LayerKind::Text {
                text: "testing".into(),
                font_size: 72.0,
                color: [255, 255, 255, 255],
            },
            ..Default::default()
        };
        let cached = LayerVisualKey::new(&layer, 1.0);
        layer.transform.scale_x = 3.0;
        layer.transform.scale_y = 3.0;
        let interactive = desired_layer_visual_key(&layer, 1.0, true, Some(&cached));
        let settled = desired_layer_visual_key(&layer, 1.0, false, Some(&cached));
        assert_eq!(interactive.text_raster_scale, 1);
        assert_eq!(settled.text_raster_scale, 4);
    }

    #[test]
    fn active_transform_reuses_clean_cached_text_without_key_work() {
        assert!(reuse_cached_visual_during_interaction(
            Some(2048),
            1024,
            false,
            true
        ));
        assert!(!reuse_cached_visual_during_interaction(
            Some(2048),
            1024,
            true,
            true
        ));
    }

    #[test]
    fn every_corner_has_a_generous_resize_target() {
        let geometry = CanvasGeometry {
            viewport: Rect::from_min_size(Pos2::ZERO, Vec2::splat(500.0)),
            canvas: Rect::from_min_size(Pos2::ZERO, Vec2::splat(500.0)),
            pixels_per_point: 1.0,
        };
        let layer = Layer {
            kind: LayerKind::Rectangle {
                width: 100,
                height: 60,
                color: [255, 255, 255, 255],
                corner_radius: 0.0,
            },
            transform: Transform {
                x: 100.0,
                y: 80.0,
                ..Default::default()
            },
            ..Default::default()
        };
        for (pointer, expected) in [
            (Pos2::new(112.0, 92.0), ResizeHandle::TopLeft),
            (Pos2::new(188.0, 92.0), ResizeHandle::TopRight),
            (Pos2::new(112.0, 128.0), ResizeHandle::BottomLeft),
            (Pos2::new(188.0, 128.0), ResizeHandle::BottomRight),
        ] {
            assert_eq!(
                resize_handle_at(geometry, &layer, None, pointer),
                Some(expected)
            );
        }
        assert_eq!(
            resize_cursor(ResizeHandle::TopLeft),
            egui::CursorIcon::ResizeNwSe
        );
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

    #[test]
    fn action_search_matches_intent_not_just_tool_names() {
        assert!(Tool::Move.matches("resize"));
        assert!(Tool::Mask.matches("visible region"));
        assert!(!Tool::Text.matches("crop"));
    }

    #[test]
    fn tools_declare_whether_selection_opens_ui_or_arms_the_canvas() {
        assert_eq!(Tool::Text.activation(), ToolActivation::ImmediateDialog);
        assert_eq!(Tool::Shape.activation(), ToolActivation::ChoiceDialog);
        assert_eq!(Tool::Crop.activation(), ToolActivation::CanvasGesture);
    }
}
