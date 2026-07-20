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
    Alignment, AlignmentReference, BlendMode, Command, Document, GuideOrientation, Layer,
    LayerKind, LayerMask, ShapeStroke, Transform, Workspace, export_document,
};
use spectrum_imaging::AdjustmentPatch;

#[path = "prism_gui/alignment.rs"]
mod alignment;
#[path = "prism_gui/canvas.rs"]
mod canvas;
use alignment::*;
#[path = "prism_gui/chrome.rs"]
mod chrome;
#[path = "prism_gui/compositor.rs"]
mod compositor;
#[path = "prism_gui/dialogs.rs"]
mod dialogs;
#[path = "prism_gui/history.rs"]
mod history;
#[path = "prism_gui/inline_text.rs"]
mod inline_text;
#[path = "prism_gui/inspector.rs"]
mod inspector;
#[path = "prism_gui/layers.rs"]
mod layers;
#[cfg(target_os = "macos")]
#[path = "prism_gui/macos.rs"]
mod macos;
#[path = "prism_gui/model.rs"]
mod model;
use model::*;
#[path = "prism_gui/project_lifecycle.rs"]
mod project_lifecycle;
#[path = "prism_gui/renderer.rs"]
mod renderer;
#[path = "prism_gui/shortcuts.rs"]
mod shortcuts;
use compositor::*;
#[path = "prism_gui/terminal.rs"]
mod terminal;
#[path = "prism_gui/terminal_input.rs"]
mod terminal_input;
#[path = "prism_gui/terminal_render.rs"]
mod terminal_render;
#[path = "prism_gui/theme.rs"]
mod theme;
#[path = "prism_gui/typography_ui.rs"]
mod typography_ui;
use history::HistoryViewState;
use project_lifecycle::MoveProjectDialog;
use renderer::*;
use terminal::TerminalDock;
use theme::*;

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
    composition_scroll_to_selection: Option<u64>,
    zoom: f32,
    pan: Vec2,
    fit_requested: bool,
    drag: Option<DragState>,
    smart_guides: SmartGuides,
    alignment_reference: Option<u64>,
    rename_layer: Option<(u64, String)>,
    new_dialog: Option<NewDocumentDialog>,
    inline_text_editor: Option<inline_text::InlineTextEditor>,
    font_query: String,
    delete_confirmation: Option<u64>,
    layer_drag: Option<u64>,
    layer_drop_index: Option<usize>,
    move_project_dialog: Option<MoveProjectDialog>,
    history: HistoryViewState,
    open_document_receiver: Receiver<PathBuf>,
    #[cfg(target_os = "macos")]
    native_menu: macos::NativeMenuBridge,
    pending_open_documents: VecDeque<(std::time::Instant, PathBuf)>,
    startup_project_ready_at: std::time::Instant,
    collaboration_poll_at: std::time::Instant,
    workspace_initialized: bool,
    terminal: TerminalDock,
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
    macos::run(initial_project)
}

impl PrismApp {
    fn new(
        creation: &eframe::CreationContext<'_>,
        initial_project: Option<&Path>,
        open_document_receiver: Receiver<PathBuf>,
        #[cfg(target_os = "macos")] native_menu: macos::NativeMenuBridge,
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
            composition_scroll_to_selection: None,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit_requested: true,
            drag: None,
            smart_guides: SmartGuides::default(),
            alignment_reference: None,
            rename_layer: None,
            new_dialog: None,
            inline_text_editor: None,
            font_query: String::new(),
            delete_confirmation: None,
            layer_drag: None,
            layer_drop_index: None,
            move_project_dialog: None,
            history: HistoryViewState::new(creation.egui_ctx.clone()),
            open_document_receiver,
            #[cfg(target_os = "macos")]
            native_menu,
            pending_open_documents: VecDeque::new(),
            startup_project_ready_at: std::time::Instant::now()
                + std::time::Duration::from_millis(250),
            collaboration_poll_at: std::time::Instant::now(),
            workspace_initialized,
            terminal: TerminalDock::default(),
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
        if self.inline_text_editor.is_some() && !matches!(command, Command::SelectLayer { .. }) {
            self.settle_inline_text_editor();
        }
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
                self.history.mark_stale();
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
            self.settle_inline_text_editor();
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
        self.settle_inline_text_editor();
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
        self.drag = None;
        self.smart_guides = SmartGuides::default();
        self.alignment_reference = None;
    }

    fn activate_tab(&mut self, id: u64) {
        if id == self.active_tab_id {
            return;
        }
        self.settle_inline_text_editor();
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
        self.smart_guides = SmartGuides::default();
        self.alignment_reference = None;
    }

    fn close_tab(&mut self, id: u64) {
        self.settle_inline_text_editor();
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
        self.smart_guides = SmartGuides::default();
        self.alignment_reference = None;
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
            .exact_size(STATUS_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(6, 3))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.spacing_mut().interact_size.y = 18.0;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&self.status)
                            .size(11.0)
                            .color(if self.status_error { DANGER } else { MUTED }),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        #[cfg(not(target_os = "macos"))]
                        self.terminal_status_control(ui);
                        self.snapping_control(ui);
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
        self.process_native_menu_actions(root.ctx());
        let context = root.ctx().clone();
        self.receive_open_documents(&context);
        self.sync_agent_collaborations(&context);
        self.poll_terminals(&context);
        self.keyboard(&context);
        if self.terminal.visible() {
            self.terminal_panel(root);
        } else {
            self.top_bar(root);
            if self.history.visible {
                self.history_view(root);
            } else {
                self.workbench_bar(root);
                self.status_bar(root);
                self.right_panel(root);
                self.canvas(root);
            }
        }
        self.dialogs(&context);
        #[cfg(target_os = "macos")]
        self.sync_native_menu_state(&context);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.settle_inline_text_editor();
        self.finish_interaction();
        let _ = self.workspace.checkpoint();
        for workspace in self.inactive_workspaces.values() {
            let _ = workspace.checkpoint();
        }
        self.terminal.shutdown();
    }
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
                typography: prism_core::TextTypography::default(),
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
                typography: prism_core::TextTypography::default(),
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
        assert_eq!(
            canvas_invalidation(&Command::SetTextTypography {
                id: 7,
                typography: prism_core::TextTypography::default(),
            }),
            CanvasInvalidation::Layer(7)
        );
        assert_eq!(
            canvas_invalidation(&Command::ImportFont {
                path: PathBuf::from("face.otf"),
            }),
            CanvasInvalidation::None
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
        assert_eq!(Tool::Text.activation(), ToolActivation::CanvasGesture);
        assert_eq!(Tool::Shape.activation(), ToolActivation::ChoiceDialog);
        assert_eq!(Tool::Crop.activation(), ToolActivation::CanvasGesture);
    }
}
