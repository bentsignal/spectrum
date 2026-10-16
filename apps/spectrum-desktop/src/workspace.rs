use crate::{controls::in_sidebar, store::Store, theme::*};
use gpui::{prelude::*, *};
use gpui_component::{
    IconName, Root,
    input::{InputEvent, InputState},
    slider::{SliderEvent, SliderState},
};
use spectrum_library::{AssetId, ProjectId};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub const SIDEBAR_WIDTH: f32 = 288.;
/// A file's modification time and length, to notice when it changes.
pub type FileStamp = (Option<std::time::SystemTime>, u64);
pub const HEADER_HEIGHT: f32 = 52.;
/// Room for the macOS window buttons at the top-left of the window.
pub const TRAFFIC_LIGHTS: f32 = if cfg!(target_os = "macos") { 86. } else { 0. };

/// Where the user is: Home, or inside one project.
#[derive(Clone, Copy, PartialEq)]
pub enum Place {
    Home,
    Project(ProjectId),
}

/// What fills the main area inside a project.
#[derive(Clone, Copy, PartialEq)]
pub enum Open {
    Overview,
    Image(AssetId),
    Canvas(AssetId),
    /// Footage or audio, in the player.
    Media(AssetId),
    /// A video, in the editor.
    Video(AssetId),
}

/// Sidebar modes are capabilities; only those that apply are offered.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Projects,
    Assets,
    Color,
    Crop,
    Layers,
    Overview,
    Style,
    Info,
    Canvas,
    History,
    /// A video's selected clips.
    Edit,
    Speed,
    /// A video's own size, frame rate, and background.
    Format,
    Media,
    Export,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Projects => "Projects",
            Mode::Assets => "Assets",
            Mode::Color => "Color",
            Mode::Crop => "Crop",
            Mode::Style => "Style",
            Mode::Info => "Info",
            Mode::Layers => "Layers",
            Mode::Overview => "Overview",
            Mode::Canvas => "Canvas",
            Mode::History => "History",
            Mode::Edit => "Clip",
            Mode::Speed => "Speed",
            Mode::Format => "Video",
            Mode::Media => "Media",
            Mode::Export => "Export",
        }
    }

    pub fn icon(self) -> gpui_component::Icon {
        let name = match self {
            Mode::History => return crate::icons::icon("history"),
            Mode::Edit => return crate::icons::icon("film"),
            Mode::Speed => return crate::icons::icon("gauge"),
            Mode::Format => IconName::Settings,
            Mode::Media => IconName::Inbox,
            Mode::Export => IconName::ExternalLink,
            Mode::Projects => IconName::FolderClosed,
            Mode::Assets => IconName::LayoutDashboard,
            Mode::Color => IconName::Sun,
            Mode::Crop => IconName::Maximize,
            Mode::Style => IconName::Palette,
            Mode::Info => IconName::Info,
            Mode::Layers => IconName::GalleryVerticalEnd,
            Mode::Overview => IconName::Inspector,
            Mode::Canvas => IconName::Frame,
        };
        gpui_component::Icon::new(name)
    }
}

/// Which library assets a grid shows.
#[derive(Clone, Copy, PartialEq)]
pub enum LibraryView {
    All,
    Unassigned,
    Trash,
    Project(ProjectId),
}

pub struct Workspace {
    pub place: Place,
    pub mode: Mode,
    pub open: Open,
    pub sidebar_right: bool,
    /// The real library, or why it could not open.
    pub store: Result<Store, SharedString>,
    pub view: LibraryView,
    pub selection: Vec<AssetId>,
    pub project_selection: Vec<ProjectId>,
    pub project_anchor: Option<ProjectId>,
    pub anchor: Option<AssetId>,
    /// Drag-box selection, in window coordinates: start and current point.
    pub marquee: Option<crate::marquee::Marquee>,
    pub autoscrolling: bool,
    pub grid_scroll: ScrollHandle,
    /// Card bounds from the last frame, for drag-box selection.
    pub card_bounds: Rc<RefCell<HashMap<AssetId, Bounds<Pixels>>>>,
    pub grid_bounds: Rc<RefCell<Bounds<Pixels>>>,
    pub palette_open: bool,
    pub palette_query: Entity<InputState>,
    pub palette_index: usize,
    pub palette_scroll: ScrollHandle,
    /// The palette lists canvas tools instead of places.
    pub palette_tools: bool,
    /// The palette lists the open asset's branches (History).
    pub palette_branches: bool,
    /// Assets the palette is adding to a project; empty when it navigates.
    pub palette_adding: Vec<AssetId>,
    /// Edits copied from an image, ready to paste onto others.
    pub copied_edits: Option<spectrum_image::Adjustments>,
    pub picker_open: bool,
    pub picker_query: Entity<InputState>,
    pub picker_selected: Vec<AssetId>,
    pub picker_anchor: Option<AssetId>,
    /// The picker places images on the open canvas instead of adding assets.
    pub picker_place: bool,
    /// The canvas layer whose image the picker replaces.
    pub picker_replace: Option<u64>,
    pub picker_assets: Vec<spectrum_library::Asset>,
    /// Assets to add to the next project created.
    pub pending_add: Vec<AssetId>,
    pub importing: usize,
    pub import_token: u64,
    pub pending_imports: Vec<crate::library::PendingImport>,
    pub search: Entity<InputState>,
    pub project_search: Entity<InputState>,
    pub new_project_name: Entity<InputState>,
    pub rename_input: Entity<InputState>,
    /// Kinds of asset the grids leave out.
    pub hidden_kinds: Vec<spectrum_library::AssetKind>,
    pub sort: usize,
    /// The image editor: color, crop, and the open image.
    pub image: crate::editors::ImageEditor,
    /// The canvas editor's controls for the selected layer and fonts.
    pub canvas_ui: crate::editors::CanvasControls,
    pub export: Entity<crate::export::ExportSettings>,
    pub canvas_size: Entity<crate::canvas_size::CanvasSize>,
    /// The sidebar's mode list is open, and whether holding Command opened it.
    pub mode_menu: bool,
    pub mode_switcher_bounds: std::rc::Rc<std::cell::RefCell<Bounds<Pixels>>>,
    pub mode_menu_held: bool,
    /// Counts Command presses and other keys, so a pending hold knows if it
    /// is still the same one.
    pub hold_token: u64,
    pub colors: crate::colors::Colors,
    /// The open footage or audio, and the player's controls.
    pub player: Option<crate::player::Player>,
    pub player_controls: Option<crate::player_view::PlayerControls>,
    /// The open video and its export choices.
    pub video: Option<crate::video_state::VideoEditor>,
    /// The open canvas, when one fills the main area.
    pub canvas: Option<crate::canvas_state::CanvasState>,
    /// A check for an agent's newer work is running.
    pub following: bool,
    /// The open asset's document file as of the last check, so unchanged
    /// files are not checked again.
    pub followed_stamp: Option<(AssetId, Option<FileStamp>)>,
    /// Two-finger scrolls on the canvas or the open image.
    pub gestures: crate::gesture::ScrollGestures,
    /// The History mode's tree, selection, and watching.
    pub history: crate::history_view::HistoryView,
    /// Where the sidebar is scrolled to.
    pub sidebar_scroll: ScrollHandle,
    /// A render is coming to free retired images (`retire`).
    retire_scheduled: bool,
    /// Keeps keyboard shortcuts working when no field has focus.
    pub focus_handle: FocusHandle,
    /// The title strip a window drag started in, if any.
    dragging: Option<&'static str>,
    /// Frames left to re-render after a layout change. GPUI Component sliders
    /// position their thumbs from the previous frame's bounds.
    settle: u8,
    _subscriptions: Vec<Subscription>,
}

pub fn slider(
    cx: &mut Context<Workspace>,
    min: f32,
    max: f32,
    step: f32,
    value: f32,
) -> Entity<SliderState> {
    let state = cx.new(|_| {
        // Set the upper bound first: `min` clamps against the current maximum.
        SliderState::new()
            .max(max)
            .min(min)
            .step(step)
            .default_value(value)
    });
    crate::controls::remember_range(&state, min, max);
    state
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut input = |placeholder: &'static str, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let search = input("Search assets", cx);
        let project_search = input("Search projects", cx);
        let palette_query = input("Go to a project, asset, or place", cx);
        let new_project_name = input("Project name", cx);
        let picker_query = input("Search your library", cx);
        let rename_input = input("Name", cx);
        let text_input = input("Text", cx);
        let font_query = input("Search fonts", cx);
        let history_name = input("Name this revision", cx);
        let opacity = in_sidebar(slider(cx, 0., 100., 1., 100.));
        let text_size = in_sidebar(slider(cx, 8., 400., 1., 48.));
        let corner = in_sidebar(slider(cx, 0., 200., 1., 0.));
        let rotation = in_sidebar(slider(cx, -180., 180., 1., 0.));
        let line_height = in_sidebar(slider(cx, 0.8, 3., 0.05, 1.25));
        let tracking = in_sidebar(slider(cx, -20., 100., 1., 0.));
        let straighten = in_sidebar(slider(cx, -45., 45., 0.1, 0.));
        let color: Vec<_> = crate::color_fields::FIELDS
            .iter()
            .map(|field| in_sidebar(slider(cx, field.min, field.max, field.step, 0.)))
            .collect();
        let mut subscriptions = vec![
            // Whenever focus is lost (a field or a list closes under it),
            // the keyboard comes back to the workspace, so its shortcuts
            // never stop answering until a click.
            cx.on_focus_lost(window, |this, window, cx| {
                this.focus_handle.focus(window);
                cx.notify();
            }),
            cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&project_search, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&picker_query, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe_in(&palette_query, window, |this, _, event, window, cx| {
                this.palette_input(event, window, cx)
            }),
            cx.subscribe_in(
                &opacity,
                window,
                |this, state, _: &SliderEvent, window, cx| {
                    let opacity = state.read(cx).value().start() / 100.;
                    this.on_selected(window, cx, |id| spectrum_canvas::Command::SetOpacity {
                        id,
                        opacity,
                    });
                },
            ),
            cx.subscribe_in(
                &text_size,
                window,
                |this, _, _: &SliderEvent, window, cx| this.update_text(None, window, cx),
            ),
            cx.subscribe_in(&corner, window, |this, _, _: &SliderEvent, window, cx| {
                this.update_shape(None, window, cx)
            }),
            cx.subscribe_in(
                &rotation,
                window,
                |this, state, _: &SliderEvent, window, cx| {
                    let degrees = state.read(cx).value().start();
                    this.on_selected(window, cx, |id| spectrum_canvas::Command::SetRotation {
                        id,
                        degrees,
                    });
                },
            ),
            cx.subscribe_in(
                &line_height,
                window,
                |this, _, _: &SliderEvent, window, cx| this.update_typography(None, window, cx),
            ),
            cx.subscribe_in(&tracking, window, |this, _, _: &SliderEvent, window, cx| {
                this.update_typography(None, window, cx)
            }),
            cx.subscribe_in(&font_query, window, |this, _, event, window, cx| {
                this.font_query_event(event, window, cx)
            }),
            cx.subscribe_in(
                &text_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    // Every edit commits; the queue keeps only the latest. Syncing the
                    // field also emits Change, so skip text the layer already has.
                    if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. })
                        && this.text_edited(cx)
                    {
                        this.update_text(None, window, cx)
                    }
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        this.stop_editing_text(window, cx);
                    }
                },
            ),
        ];
        subscriptions.push(cx.subscribe_in(
            &history_name,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.name_current_branch(window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &straighten,
            window,
            |this, state, _: &SliderEvent, window, cx| {
                this.image.adjust.straighten = state.read(cx).value().start();
                this.image.adjust.crop = None;
                this.schedule_color_edit(window, cx);
            },
        ));
        for (index, state) in color.iter().enumerate() {
            subscriptions.push(cx.subscribe_in(
                state,
                window,
                move |this, _, _: &SliderEvent, window, cx| {
                    this.color_slider_changed(index, window, cx)
                },
            ));
        }
        let mut workspace = Self {
            place: Place::Home,
            mode: Mode::Projects,
            open: Open::Overview,
            sidebar_right: false,
            store: Store::open().map_err(|e| format!("{e:#}").into()),
            view: LibraryView::All,
            selection: Vec::new(),
            project_selection: Vec::new(),
            project_anchor: None,
            anchor: None,
            marquee: None,
            autoscrolling: false,
            grid_scroll: ScrollHandle::new(),
            card_bounds: Default::default(),
            grid_bounds: Default::default(),
            palette_open: false,
            palette_query,
            palette_index: 0,
            palette_scroll: ScrollHandle::new(),
            palette_tools: false,
            palette_branches: false,
            palette_adding: Vec::new(),
            copied_edits: None,
            pending_add: Vec::new(),
            picker_open: false,
            picker_query,
            picker_selected: Vec::new(),
            picker_anchor: None,
            picker_place: false,
            picker_replace: None,
            picker_assets: Vec::new(),
            importing: 0,
            import_token: 0,
            pending_imports: Vec::new(),
            search,
            project_search,
            new_project_name,
            rename_input,
            hidden_kinds: Vec::new(),
            sort: 0,
            image: crate::editors::ImageEditor {
                color,
                adjust: Default::default(),
                color_section: 0,
                mix_band: 0,
                grade_range: 0,
                curve_channel: 0,
                curve_drag: None,
                curve_bounds: Default::default(),
                straighten,
                preview: None,
                edits: Default::default(),
                crop_drag: None,
                crop_aspect: 0,
                image_bounds: Default::default(),
                color_busy: false,
                color_dirty: false,
                photo_info: None,
                view: Default::default(),
                compare: false,
            },
            canvas_ui: crate::editors::CanvasControls {
                fonts: None,
                font_open: false,
                font_blocked: Default::default(),
                font_query,
                font_highlight: 0,
                font_hover: None,
                font_scroll: UniformListScrollHandle::new(),
                font_lists: crate::font_lists::FontLists::load(),
                fonts_requested: false,
                opacity,
                text_input,
                text_size,
                corner,
                rotation,
                line_height,
                tracking,
                styles: crate::layer_styles::StyleControls::new(window, cx),
                tool_options: crate::tool_options::ToolOptions::new(cx),
                fill_gradient: crate::gradient_editor::GradientEditor::new(
                    crate::gradient_editor::GradientTarget::Fill,
                    window,
                    cx,
                ),
                overlay_gradient: crate::gradient_editor::GradientEditor::new(
                    crate::gradient_editor::GradientTarget::Overlay,
                    window,
                    cx,
                ),
                style_section: 0,
                guides_visible: true,
            },
            export: crate::export::ExportSettings::new(window, cx),
            canvas_size: crate::canvas_size::CanvasSize::new(window, cx),
            mode_menu: false,
            mode_switcher_bounds: Default::default(),
            mode_menu_held: false,
            hold_token: 0,
            colors: crate::colors::Colors::new(window, cx),
            player: None,
            player_controls: None,
            video: None,
            canvas: None,
            following: false,
            followed_stamp: None,
            gestures: Default::default(),
            history: crate::history_view::HistoryView::new(history_name.clone()),
            sidebar_scroll: ScrollHandle::new(),
            retire_scheduled: false,
            focus_handle: cx.focus_handle(),
            dragging: None,
            settle: 1,
            _subscriptions: subscriptions,
        };
        if let Err(error) = workspace.reload() {
            workspace.store = Err(format!("{error:#}").into());
        }
        workspace.start_following(window, cx);
        workspace
    }

    /// Sidebar modes that apply to what is on screen, in shortcut order.
    pub fn modes(&self) -> Vec<Mode> {
        match (self.place, self.open) {
            (Place::Home, _) => vec![Mode::Projects, Mode::Assets],
            (_, Open::Overview) => Vec::new(),
            (_, Open::Image(_)) => vec![Mode::Color, Mode::Crop, Mode::Info, Mode::History],
            (_, Open::Media(_)) => vec![Mode::Info],
            (_, Open::Video(_)) => vec![
                Mode::Edit,
                Mode::Speed,
                Mode::Media,
                Mode::Format,
                Mode::Export,
                Mode::History,
            ],
            (_, Open::Canvas(_)) => vec![
                Mode::Overview,
                Mode::Layers,
                Mode::Style,
                Mode::Color,
                Mode::Canvas,
                Mode::History,
            ],
        }
    }

    pub fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modes().contains(&mode) {
            return;
        }
        self.mode = mode;
        // Crop shows the whole frame; other modes show the crop.
        self.switch_image_view(window, cx);
        if mode == Mode::Color && matches!(self.open, Open::Canvas(_)) {
            self.load_layer_color(window, cx);
        }
        self.settle = 1;
        if mode == Mode::History {
            self.load_history(window, cx);
        }
        if matches!(mode, Mode::Format | Mode::Export) {
            self.sync_video_settings(window, cx);
        }
        if mode == Mode::Assets && self.place == Place::Home {
            let view = match self.view {
                LibraryView::Project(_) => LibraryView::All,
                view => view,
            };
            self.show(view, window, cx);
        }
        cx.notify();
    }

    /// Delete or Backspace: removes the selected canvas layer, or asks to
    /// delete the selected assets.
    pub(crate) fn delete_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open || self.picker_open {
            return;
        }
        match self.open {
            Open::Video(_) => self.delete_video_clips(false, window, cx),
            Open::Canvas(_) if self.has_canvas_selection() => self.delete_in_selection(window, cx),
            Open::Canvas(_) => self.on_selected(window, cx, |id| {
                spectrum_canvas::Command::RemoveLayer { id }
            }),
            Open::Overview if !self.selection.is_empty() && self.view != LibraryView::Trash => {
                let ids = self.selection.clone();
                self.confirm_delete(ids, window, cx);
            }
            _ => {}
        }
    }

    /// Re-render once more so sliders shown for the first time lay out.
    pub fn settle_next_frame(&mut self) {
        self.settle = 1;
    }

    /// Command+1 to 9: the nth mode offered.
    pub fn nth_mode(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mode) = self.modes().get(index).copied() {
            self.set_mode(mode, window, cx);
        }
    }

    /// Option+1 to 6: the nth section of a mode split into sections.
    pub fn nth_section(&mut self, index: usize, cx: &mut Context<Self>) {
        let (slot, count) = match self.mode {
            Mode::Color => (
                &mut self.image.color_section,
                crate::color_fields::SECTIONS.len(),
            ),
            Mode::Style => (
                &mut self.canvas_ui.style_section,
                crate::canvas_style::SECTIONS.len(),
            ),
            _ => return,
        };
        if index < count {
            *slot = index;
            self.settle = 1;
            cx.notify();
        }
    }

    pub fn go_home(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.save_now(window, cx);
        self.place = Place::Home;
        self.open = Open::Overview;
        self.mode = mode;
        self.settle = 1;
        let view = match self.view {
            LibraryView::Project(_) => LibraryView::All,
            view => view,
        };
        self.show(view, window, cx);
    }

    pub fn enter_project(&mut self, id: ProjectId, window: &mut Window, cx: &mut Context<Self>) {
        self.save_now(window, cx);
        self.place = Place::Project(id);
        self.open = Open::Overview;
        self.mode = Mode::Assets;
        self.settle = 1;
        self.show(LibraryView::Project(id), window, cx);
    }

    /// Shows an item in the main area and picks the mode that fits it.
    pub fn open_item(&mut self, open: Open, window: &mut Window, cx: &mut Context<Self>) {
        let _timer = crate::perf::Timer::new("main_open_item");
        self.save_now(window, cx);
        self.open = open;
        self.settle = 1;
        // Closing the player or the editor stops its sound.
        self.player = None;
        self.video = None;
        self.mode = match open {
            Open::Overview => Mode::Assets,
            Open::Image(id) => {
                self.load_color(id, window, cx);
                Mode::Color
            }
            Open::Canvas(id) => {
                self.load_canvas(id, window, cx);
                Mode::Overview
            }
            Open::Media(id) => {
                self.load_player(id, window, cx);
                Mode::Info
            }
            Open::Video(id) => {
                self.load_video(id, window, cx);
                Mode::Edit
            }
        };
        cx.notify();
    }

    /// Create a project from the name field. Returns false to keep the dialog open.
    pub fn create_project(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let name = self.new_project_name.read(cx).value().to_string();
        let Ok(store) = &mut self.store else {
            return false;
        };
        let adding = std::mem::take(&mut self.pending_add);
        let created = store
            .service
            .library
            .create_project(&name)
            .and_then(|project| {
                store.service.library.add_to_project(project.id, &adding)?;
                Ok(project)
            });
        match created {
            Ok(project) if adding.is_empty() => {
                self.enter_project(project.id, window, cx);
                true
            }
            Ok(project) => {
                self.added_notice(adding.len(), project.id, &project.name, window, cx);
                self.change(window, cx, |_| Ok(()));
                true
            }
            Err(error) => {
                self.notify_error(error, window, cx);
                false
            }
        }
    }

    /// A strip that moves the window when dragged, like a native title bar.
    pub fn drag_area(&self, id: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event, window, _| {
                    if !crate::titlebar::press(event, window) {
                        this.dragging = Some(id);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = None),
            )
            // A release or press elsewhere (after a double-click zoom or a
            // system window move) must not leave a stale drag that a later
            // slider drag across this strip would pick up.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |this, _, _, _| {
                    if this.dragging == Some(id) {
                        this.dragging = None;
                    }
                }),
            )
            .on_mouse_down_out(cx.listener(move |this, _, _, _| {
                if this.dragging == Some(id) {
                    this.dragging = None;
                }
            }))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, window, _| {
                if this.dragging != Some(id) {
                    return;
                }
                this.dragging = None;
                if event.pressed_button == Some(MouseButton::Left) {
                    window.start_window_move();
                }
            }))
    }

    /// Goes up one level: from an open item to the project, then to Home.
    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match (self.place, self.open) {
            (Place::Project(_), Open::Overview) => self.go_home(Mode::Projects, window, cx),
            (Place::Project(_), _) => self.open_item(Open::Overview, window, cx),
            (Place::Home, _) => {}
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timer = crate::perf::Timer::new("main_render");
        if self.settle > 0 {
            self.settle -= 1;
            window.request_animation_frame();
        }
        let viewport = window.viewport_size();
        let area = size(
            f32::from(viewport.width) - SIDEBAR_WIDTH,
            f32::from(viewport.height) - HEADER_HEIGHT,
        );
        // Thumbnails render only where they show, so editing never competes
        // with redrawing grids nobody can see.
        if self.thumbnails_shown() {
            self.request_thumbnails(cx);
        }
        self.request_clip_visuals(cx);
        // Nothing focused at all: the keyboard belongs to the workspace.
        if window.focused(cx).is_none() {
            self.focus_handle.focus(window);
        }
        self.ensure_preview(window, cx);
        self.fit_canvas_resolution(window, cx);
        self.step_zoom_animation(window, cx);
        self.step_image_animation(window);
        self.step_player(window, cx);
        self.prepare_copies(cx);
        crate::retire::flush(window);
        if crate::retire::waiting() && !self.retire_scheduled {
            // Come back to free the images still waiting.
            self.retire_scheduled = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                this.update(cx, |this, cx| {
                    this.retire_scheduled = false;
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        let content = match (self.place, self.open) {
            (Place::Home, _) if self.mode == Mode::Projects => {
                self.projects_grid(area.width, cx).into_any_element()
            }
            (Place::Home, _) | (_, Open::Overview) => {
                self.asset_grid(area.width, cx).into_any_element()
            }
            (_, Open::Image(_) | Open::Canvas(_) | Open::Video(_))
                if self.mode == Mode::History =>
            {
                self.history_main(window, cx)
            }
            (_, Open::Image(_)) if self.mode == Mode::Crop => self.crop_view(cx).into_any_element(),
            (_, Open::Image(_)) => self.image_view(window, cx).into_any_element(),
            (_, Open::Canvas(_)) => self.canvas_main(window, cx).into_any_element(),
            (_, Open::Media(_)) => self.player_main(window, cx),
            (_, Open::Video(_)) => self.video_main(window, cx),
        };
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(self.header(window, cx))
            .child(div().flex_1().min_h_0().child(content))
            .drag_over::<ExternalPaths>(|el, _, _, _| el.bg(rgb(0x141414)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.import_paths(paths.paths().to_vec(), window, cx)
            }));
        let sidebar = self.sidebar(window, cx).into_any_element();
        self.zoom_actions(div(), cx)
            .size_full()
            .flex()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .track_focus(&self.focus_handle)
            // A dragged layer row can be released anywhere in the window.
            .on_drop(
                cx.listener(|this, row: &crate::canvas_layers::LayerRow, window, cx| {
                    this.drop_layer(row, window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if window.focused(cx).is_none() {
                        this.focus_handle.focus(window);
                    }
                }),
            )
            .map(|el| self.shortcuts(el, cx))
            .map(|el| {
                if self.sidebar_right {
                    el.child(main).child(sidebar)
                } else {
                    el.child(sidebar).child(main)
                }
            })
            .when(self.palette_open, |el| el.child(self.palette(cx)))
            .when(self.picker_open, |el| el.child(self.picker(cx)))
            .children(crate::controls::warm_sliders())
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
            .map(|el| {
                // New sliders were drawn clear; draw again now that they have a width.
                if crate::controls::take_slider_frame() {
                    window.request_animation_frame();
                }
                el
            })
    }
}
