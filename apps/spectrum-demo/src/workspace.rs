use crate::{
    ClearSelection, Mode1, Mode2, Mode3, Mode4, Mode5, OpenPalette, SelectAll, ZoomIn, ZoomOut,
    color,
    samples::{self, Asset, Look},
    store::Store,
    theme::*,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, InteractiveElementExt, Root, Sizable,
    button::{Button, ButtonVariants},
    input::{InputEvent, InputState},
    slider::{SliderEvent, SliderState},
};
use spectrum_library::{AssetId, ProjectId};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub const SIDEBAR_WIDTH: f32 = 288.;
pub const HEADER_HEIGHT: f32 = 52.;
/// Room for the macOS window buttons at the top-left of the window.
const TRAFFIC_LIGHTS: f32 = if cfg!(target_os = "macos") { 86. } else { 0. };

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
    /// The sample canvas, until canvases are editable in this app.
    Sample,
}

/// Sidebar modes are capabilities; only those that apply are offered.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Projects,
    Assets,
    Color,
    Layers,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Projects => "Projects",
            Mode::Assets => "Assets",
            Mode::Color => "Color",
            Mode::Layers => "Layers",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Mode::Projects => IconName::FolderClosed,
            Mode::Assets => IconName::LayoutDashboard,
            Mode::Color => IconName::Sun,
            Mode::Layers => IconName::GalleryVerticalEnd,
        }
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

pub struct Layer {
    pub name: &'static str,
    pub icon: IconName,
    pub visible: bool,
    pub opacity: f32,
    pub blend: usize,
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
    /// Assets the palette is adding to a project; empty when it navigates.
    pub palette_adding: Vec<AssetId>,
    pub picker_open: bool,
    pub picker_query: Entity<InputState>,
    pub picker_selected: Vec<AssetId>,
    pub picker_anchor: Option<AssetId>,
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
    pub show_images: bool,
    pub show_canvases: bool,
    pub sort: usize,
    /// Index into `grid::ZOOM`, the asset card width.
    pub zoom: usize,
    /// Real color correction sliders, in `color::FIELDS` order.
    pub color: Vec<Entity<SliderState>>,
    /// A color edit is running; `color_dirty` asks for another when it ends.
    pub color_busy: bool,
    pub color_dirty: bool,
    /// Sample content for the sample canvas.
    pub assets: Vec<Asset>,
    pub exposure: Entity<SliderState>,
    pub contrast: Entity<SliderState>,
    pub temperature: Entity<SliderState>,
    pub saturation: Entity<SliderState>,
    pub layers: Vec<Layer>,
    pub layer: usize,
    pub opacity: Entity<SliderState>,
    pub global: bool,
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
    cx.new(|_| {
        // Set the upper bound first: `min` clamps against the current maximum.
        SliderState::new()
            .max(max)
            .min(min)
            .step(step)
            .default_value(value)
    })
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
        let exposure = slider(cx, -2., 2., 0.05, 0.);
        let contrast = slider(cx, -100., 100., 1., 0.);
        let temperature = slider(cx, -100., 100., 1., 0.);
        let saturation = slider(cx, -100., 100., 1., 0.);
        let opacity = slider(cx, 0., 100., 1., 100.);
        let color: Vec<_> = color::FIELDS
            .iter()
            .map(|field| slider(cx, field.min, field.max, field.step, 0.))
            .collect();
        let mut subscriptions = vec![
            cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&project_search, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&picker_query, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe_in(&palette_query, window, |this, _, event, window, cx| {
                this.palette_input(event, window, cx)
            }),
            cx.subscribe(&opacity, |this, state, _: &SliderEvent, cx| {
                let layer = this.layer;
                this.layers[layer].opacity = state.read(cx).value().start();
                cx.notify();
            }),
        ];
        for state in [&exposure, &contrast, &temperature, &saturation] {
            subscriptions.push(cx.subscribe(state, |this, _, _: &SliderEvent, cx| {
                let look = this.sample_look(cx);
                if let Some(target) = this.sample_target() {
                    this.assets[target].look = look;
                }
                cx.notify();
            }));
        }
        for state in &color {
            subscriptions.push(cx.subscribe_in(
                state,
                window,
                |this, _, _: &SliderEvent, window, cx| this.schedule_color_edit(window, cx),
            ));
        }
        let layer = |name, icon| Layer {
            name,
            icon,
            visible: true,
            opacity: 100.,
            blend: 0,
        };
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
            palette_adding: Vec::new(),
            pending_add: Vec::new(),
            picker_open: false,
            picker_query,
            picker_selected: Vec::new(),
            picker_anchor: None,
            picker_assets: Vec::new(),
            importing: 0,
            import_token: 0,
            pending_imports: Vec::new(),
            search,
            project_search,
            new_project_name,
            rename_input,
            show_images: true,
            show_canvases: true,
            sort: 0,
            zoom: crate::grid::DEFAULT_ZOOM,
            color,
            color_busy: false,
            color_dirty: false,
            assets: samples::library(),
            exposure,
            contrast,
            temperature,
            saturation,
            layers: vec![
                layer("Title", IconName::ALargeSmall),
                layer("Harbor at dusk", IconName::Frame),
                layer("Background", IconName::Frame),
            ],
            layer: 0,
            opacity,
            global: false,
            focus_handle: cx.focus_handle(),
            dragging: None,
            settle: 1,
            _subscriptions: subscriptions,
        };
        if let Err(error) = workspace.reload() {
            workspace.store = Err(format!("{error:#}").into());
        }
        workspace
    }

    /// Sidebar modes that apply to what is on screen, in shortcut order.
    pub fn modes(&self) -> Vec<Mode> {
        match (self.place, self.open) {
            (Place::Home, _) => vec![Mode::Projects, Mode::Assets],
            (_, Open::Overview) => Vec::new(),
            (_, Open::Image(_)) => vec![Mode::Color],
            (_, Open::Sample) => vec![Mode::Layers, Mode::Color],
        }
    }

    pub fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modes().contains(&mode) {
            return;
        }
        self.mode = mode;
        self.settle = 1;
        if mode == Mode::Assets && self.place == Place::Home {
            let view = match self.view {
                LibraryView::Project(_) => LibraryView::All,
                view => view,
            };
            self.show(view, window, cx);
        }
        cx.notify();
    }

    /// Command+1 to 9: the nth mode offered.
    pub fn nth_mode(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mode) = self.modes().get(index).copied() {
            self.set_mode(mode, window, cx);
        }
    }

    pub fn go_home(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
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
        self.place = Place::Project(id);
        self.open = Open::Overview;
        self.mode = Mode::Assets;
        self.settle = 1;
        self.show(LibraryView::Project(id), window, cx);
    }

    /// Shows an item in the main area and picks the mode that fits it.
    pub fn open_item(&mut self, open: Open, window: &mut Window, cx: &mut Context<Self>) {
        self.open = open;
        self.settle = 1;
        self.mode = match open {
            Open::Overview => Mode::Assets,
            Open::Image(id) => {
                self.load_color(id, window, cx);
                Mode::Color
            }
            Open::Sample => Mode::Layers,
        };
        cx.notify();
    }

    /// The sample asset whose look the sample Color mode edits.
    pub fn sample_target(&self) -> Option<usize> {
        match self.layer {
            1 => Some(0),
            2 => Some(1),
            _ => None,
        }
    }

    fn sample_look(&self, cx: &App) -> Look {
        let value = |state: &Entity<SliderState>| state.read(cx).value().start();
        Look {
            exposure: value(&self.exposure),
            contrast: value(&self.contrast),
            temperature: value(&self.temperature),
            saturation: value(&self.saturation),
        }
    }

    pub fn select_layer(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.layer = index;
        let opacity = self.layers[index].opacity;
        self.opacity
            .update(cx, |state, cx| state.set_value(opacity, window, cx));
        if let Some(target) = self.sample_target() {
            let look = self.assets[target].look;
            for (state, value) in [
                (&self.exposure, look.exposure),
                (&self.contrast, look.contrast),
                (&self.temperature, look.temperature),
                (&self.saturation, look.saturation),
            ] {
                state.update(cx, |state, cx| state.set_value(value, window, cx));
            }
        }
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
    fn drag_area(&self, id: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .on_double_click(|_, window, _| {
                if cfg!(target_os = "macos") {
                    window.titlebar_double_click();
                } else {
                    window.zoom_window();
                }
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, _| this.dragging = Some(id)),
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

    /// The fixed strip: a back button inside projects, then the mode tabs.
    fn strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let modes = self.modes();
        let back: Option<SharedString> = match (self.place, self.open) {
            (Place::Home, _) => None,
            (Place::Project(_), Open::Overview) => Some("Home".into()),
            (Place::Project(id), _) => self
                .store
                .as_ref()
                .ok()
                .and_then(|s| s.project_name(id))
                .map(|name| name.to_string().into()),
        };
        let selected = modes.iter().position(|m| *m == self.mode).unwrap_or(0);
        let view = cx.entity();
        let tabs = modes.clone();
        div()
            .px_3()
            .pb_3()
            .flex()
            .flex_col()
            .gap_2()
            .when(back.is_some() || !modes.is_empty(), |el| {
                el.border_b_1().border_color(rgb(BORDER))
            })
            .children(back.map(|label| {
                div()
                    .id("back")
                    // Matches the mode tabs' height so the divider stays put.
                    .h(px(36.))
                    .px_1()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .rounded_md()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .hover(|el| el.bg(rgb(HOVER)).text_color(rgb(TEXT)))
                    .child(Icon::new(IconName::ChevronLeft).small())
                    .child(div().truncate().child(label))
                    .on_click(cx.listener(|this, _, window, cx| this.back(window, cx)))
            }))
            .when(!modes.is_empty(), |el| {
                el.child(crate::controls::segmented(
                    "modes",
                    modes.iter().map(|m| (Some(m.icon()), m.label())),
                    selected,
                    move |index, window, cx| {
                        let mode = tabs[index];
                        view.update(cx, |this, cx| this.set_mode(mode, window, cx));
                    },
                ))
            })
    }

    fn sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match (self.place, self.mode) {
            (Place::Home, Mode::Projects) => self.projects_sidebar(cx).into_any_element(),
            (Place::Home, _) => self.library_sidebar(cx).into_any_element(),
            (_, _) if self.open == Open::Overview => self.project_sidebar(cx).into_any_element(),
            (_, Mode::Color) if self.open == Open::Sample => {
                self.sample_color_sidebar(cx).into_any_element()
            }
            (_, Mode::Color) => self.color_sidebar(cx).into_any_element(),
            _ => self.canvas_sidebar(window, cx).into_any_element(),
        };
        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(SIDEBAR))
            .map(|el| {
                if self.sidebar_right {
                    el.border_l_1()
                } else {
                    el.border_r_1()
                }
            })
            .border_color(rgb(BORDER))
            .child(
                self.drag_area("sidebar-header", cx)
                    .pl(px(if self.sidebar_right {
                        16.
                    } else {
                        TRAFFIC_LIGHTS
                    }))
                    .pr_3()
                    .justify_end()
                    .child(
                        Button::new("sidebar-side")
                            .ghost()
                            .small()
                            .icon(if self.sidebar_right {
                                IconName::PanelLeft
                            } else {
                                IconName::PanelRight
                            })
                            .tooltip(if self.sidebar_right {
                                "Move sidebar left"
                            } else {
                                "Move sidebar right"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_right = !this.sidebar_right;
                                cx.notify();
                            })),
                    ),
            )
            .child(self.strip(cx))
            .child(
                div()
                    .id(SharedString::from(format!("sidebar-{}", self.mode.label())))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_4()
                    .pt_4()
                    .pb_6()
                    .child(content),
            )
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = |n: usize, noun: &str| -> SharedString {
            format!("{n} {noun}{}", if n == 1 { "" } else { "s" }).into()
        };
        let entries = self.store.as_ref().map_or(0, |s| s.entries.len());
        let (title, detail, grid): (SharedString, SharedString, bool) =
            match (self.place, self.open) {
                (Place::Home, _) if self.mode == Mode::Projects => {
                    let n = self.store.as_ref().map_or(0, |s| s.projects.len());
                    ("Projects".into(), count(n, "project"), false)
                }
                (Place::Home, _) => (self.view_name(), count(entries, "asset"), true),
                (_, Open::Overview) => (self.view_name(), count(entries, "asset"), true),
                (_, Open::Image(id)) => (self.asset_name(id), "Image".into(), false),
                (_, Open::Sample) => ("Spring poster".into(), "Sample canvas".into(), false),
            };
        // Only the title area moves the window, so controls here drag normally.
        div()
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .pr_6()
            .child(
                self.drag_area("main-header", cx)
                    .flex_1()
                    .min_w_0()
                    .pl(px(if self.sidebar_right {
                        TRAFFIC_LIGHTS.max(24.)
                    } else {
                        24.
                    }))
                    .gap_3()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                    .child(div().text_sm().text_color(rgb(FAINT)).child(detail)),
            )
            .when(grid, |el| el.child(self.grid_controls(cx)))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.settle > 0 {
            self.settle -= 1;
            window.request_animation_frame();
        }
        let viewport = window.viewport_size();
        let area = size(
            f32::from(viewport.width) - SIDEBAR_WIDTH,
            f32::from(viewport.height) - HEADER_HEIGHT,
        );
        self.request_thumbnails(cx);
        self.request_large(cx);
        let content = match (self.place, self.open) {
            (Place::Home, _) if self.mode == Mode::Projects => {
                self.projects_grid(area.width, cx).into_any_element()
            }
            (Place::Home, _) | (_, Open::Overview) => {
                self.asset_grid(area.width, cx).into_any_element()
            }
            (_, Open::Image(id)) => self.image_view(id, cx).into_any_element(),
            (_, Open::Sample) => self.canvas(area, cx).into_any_element(),
        };
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(self.header(cx))
            .child(div().flex_1().min_h_0().child(content))
            .drag_over::<ExternalPaths>(|el, _, _, _| el.bg(rgb(0x141414)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.import_paths(paths.paths().to_vec(), window, cx)
            }));
        let sidebar = self.sidebar(window, cx).into_any_element();
        div()
            .size_full()
            .flex()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if window.focused(cx).is_none() {
                        this.focus_handle.focus(window);
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &Mode1, window, cx| this.nth_mode(0, window, cx)))
            .on_action(cx.listener(|this, _: &Mode2, window, cx| this.nth_mode(1, window, cx)))
            .on_action(cx.listener(|this, _: &Mode3, window, cx| this.nth_mode(2, window, cx)))
            .on_action(cx.listener(|this, _: &Mode4, window, cx| this.nth_mode(3, window, cx)))
            .on_action(cx.listener(|this, _: &Mode5, window, cx| this.nth_mode(4, window, cx)))
            .on_action(cx.listener(|this, _: &OpenPalette, window, cx| {
                if this.palette_open {
                    this.close_palette(cx)
                } else {
                    this.open_palette(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_by(1, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_by(-1, cx)))
            .on_action(cx.listener(|this, _: &ClearSelection, window, cx| {
                if this.picker_open {
                    this.close_picker(window, cx)
                } else if this.palette_open {
                    this.close_palette(cx)
                } else {
                    this.clear_selection(cx)
                }
            }))
            .map(|el| {
                if self.sidebar_right {
                    el.child(main).child(sidebar)
                } else {
                    el.child(sidebar).child(main)
                }
            })
            .when(self.palette_open, |el| el.child(self.palette(cx)))
            .when(self.picker_open, |el| el.child(self.picker(cx)))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}
