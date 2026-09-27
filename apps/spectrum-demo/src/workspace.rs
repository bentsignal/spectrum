use crate::{
    controls,
    samples::{self, Asset, Look, Project},
    theme::*,
};
use gpui::{prelude::*, *};
use gpui_component::{
    IconName, InteractiveElementExt, Root, Sizable,
    button::{Button, ButtonVariants},
    input::{InputEvent, InputState},
    slider::{SliderEvent, SliderState},
};

pub const SIDEBAR_WIDTH: f32 = 288.;
pub const HEADER_HEIGHT: f32 = 52.;
/// Room for the macOS window buttons at the top-left of the window.
const TRAFFIC_LIGHTS: f32 = if cfg!(target_os = "macos") { 86. } else { 0. };

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Library,
    Adjust,
    Canvas,
}

/// What the library grid shows.
#[derive(Clone, Copy, PartialEq)]
pub enum LibraryView {
    All,
    Unassigned,
    Project(usize),
}

pub struct Layer {
    pub name: &'static str,
    pub icon: IconName,
    pub visible: bool,
    pub opacity: f32,
    pub blend: usize,
}

pub struct Workspace {
    pub mode: Mode,
    pub sidebar_right: bool,
    pub assets: Vec<Asset>,
    pub projects: Vec<Project>,
    pub view: LibraryView,
    pub selected: usize,
    pub search: Entity<InputState>,
    pub new_project_name: Entity<InputState>,
    pub show_images: bool,
    pub show_canvases: bool,
    pub sort: usize,
    pub thumbnail: Entity<SliderState>,
    pub exposure: Entity<SliderState>,
    pub contrast: Entity<SliderState>,
    pub temperature: Entity<SliderState>,
    pub saturation: Entity<SliderState>,
    pub compare: bool,
    pub layers: Vec<Layer>,
    pub layer: usize,
    pub opacity: Entity<SliderState>,
    pub global: bool,
    dragging: bool,
    /// Frames left to re-render after a layout change. GPUI Component sliders
    /// position their thumbs from the previous frame's bounds.
    settle: u8,
    _subscriptions: Vec<Subscription>,
}

fn slider(
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
        let (assets, projects) = samples::library();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let new_project_name = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
        let thumbnail = slider(cx, 140., 280., 1., 196.);
        let exposure = slider(cx, -2., 2., 0.05, 0.);
        let contrast = slider(cx, -100., 100., 1., 0.);
        let temperature = slider(cx, -100., 100., 1., 0.);
        let saturation = slider(cx, -100., 100., 1., 0.);
        let opacity = slider(cx, 0., 100., 1., 100.);
        let mut subscriptions = vec![
            cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&thumbnail, |_, _, _: &SliderEvent, cx| cx.notify()),
            cx.subscribe(&opacity, |this, state, _: &SliderEvent, cx| {
                let layer = this.layer;
                this.layers[layer].opacity = state.read(cx).value().start();
                cx.notify();
            }),
        ];
        for state in [&exposure, &contrast, &temperature, &saturation] {
            subscriptions.push(cx.subscribe(state, |this, _, _: &SliderEvent, cx| {
                let look = this.look(cx);
                this.assets[this.selected].look = look;
                cx.notify();
            }));
        }
        let layer = |name, icon, blend| Layer {
            name,
            icon,
            visible: true,
            opacity: 100.,
            blend,
        };
        Self {
            mode: Mode::Library,
            sidebar_right: false,
            assets,
            projects,
            view: LibraryView::Project(0),
            selected: 0,
            search,
            new_project_name,
            show_images: true,
            show_canvases: true,
            sort: 0,
            thumbnail,
            exposure,
            contrast,
            temperature,
            saturation,
            compare: false,
            layers: vec![
                layer("Title", IconName::ALargeSmall, 0),
                layer("Harbor at dusk", IconName::Frame, 0),
                layer("Background", IconName::Frame, 0),
            ],
            layer: 0,
            opacity,
            global: false,
            dragging: false,
            settle: 1,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.settle = 1;
        cx.notify();
    }

    /// The look described by the adjustment sliders.
    fn look(&self, cx: &App) -> Look {
        let value = |state: &Entity<SliderState>| state.read(cx).value().start();
        Look {
            exposure: value(&self.exposure),
            contrast: value(&self.contrast),
            temperature: value(&self.temperature),
            saturation: value(&self.saturation),
        }
    }

    /// Select an asset and load its look into the adjustment sliders.
    pub fn select_asset(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = index;
        self.load_look(window, cx);
        cx.notify();
    }

    pub fn load_look(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let look = self.assets[self.selected].look;
        for (state, value) in [
            (&self.exposure, look.exposure),
            (&self.contrast, look.contrast),
            (&self.temperature, look.temperature),
            (&self.saturation, look.saturation),
        ] {
            state.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    pub fn select_layer(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.layer = index;
        let opacity = self.layers[index].opacity;
        self.opacity
            .update(cx, |state, cx| state.set_value(opacity, window, cx));
        cx.notify();
    }

    /// Add a sample project from the name field. Returns false if it is empty.
    pub fn create_project(&mut self, cx: &mut Context<Self>) -> bool {
        let name = self.new_project_name.read(cx).value().trim().to_string();
        let taken = self
            .projects
            .iter()
            .any(|p| p.name.to_lowercase() == name.to_lowercase());
        if name.is_empty() || taken {
            return false;
        }
        self.projects.push(Project {
            name: name.into(),
            assets: Vec::new(),
        });
        self.view = LibraryView::Project(self.projects.len() - 1);
        self.mode = Mode::Library;
        cx.notify();
        true
    }

    fn header_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            }))
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
                cx.listener(|this, _, _, _| this.dragging = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = false),
            )
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.dragging {
                    this.dragging = false;
                    window.start_window_move();
                }
            }))
    }

    fn sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let modes = [
            (Some(IconName::LayoutDashboard), "Library"),
            (Some(IconName::Sun), "Adjust"),
            (Some(IconName::Frame), "Canvas"),
        ];
        let selected = self.mode as usize;
        let view = cx.entity();
        let content = match self.mode {
            Mode::Library => self.library_sidebar(cx).into_any_element(),
            Mode::Adjust => self.adjust_sidebar(cx).into_any_element(),
            Mode::Canvas => self.canvas_sidebar(window, cx).into_any_element(),
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
                    .child(self.header_actions(cx)),
            )
            .child(div().px_3().child(controls::segmented(
                "mode",
                modes,
                selected,
                move |index, _, cx| {
                    let mode = [Mode::Library, Mode::Adjust, Mode::Canvas][index];
                    view.update(cx, |this, cx| this.set_mode(mode, cx));
                },
            )))
            .child(
                div()
                    .id(("sidebar-body", selected))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_4()
                    .pt_5()
                    .pb_6()
                    .child(content),
            )
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, detail): (SharedString, SharedString) = match self.mode {
            Mode::Library => {
                let count = self.view_assets().len();
                (
                    self.view_name(),
                    format!("{count} asset{}", if count == 1 { "" } else { "s" }).into(),
                )
            }
            Mode::Adjust => {
                let asset = &self.assets[self.selected];
                (asset.name.clone(), asset.dimensions.into())
            }
            Mode::Canvas => ("Spring poster".into(), "1920 × 1080".into()),
        };
        self.drag_area("main-header", cx)
            .pl(px(if self.sidebar_right {
                TRAFFIC_LIGHTS.max(24.)
            } else {
                24.
            }))
            .pr_6()
            .gap_3()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
            .child(div().text_sm().text_color(rgb(FAINT)).child(detail))
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
        let content = match self.mode {
            Mode::Library => self.library(cx).into_any_element(),
            Mode::Adjust => self.adjust(area).into_any_element(),
            Mode::Canvas => self.canvas(area, cx).into_any_element(),
        };
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(self.header(cx))
            .child(div().flex_1().min_h_0().child(content));
        let sidebar = self.sidebar(window, cx).into_any_element();
        div()
            .size_full()
            .flex()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .map(|el| {
                if self.sidebar_right {
                    el.child(main).child(sidebar)
                } else {
                    el.child(sidebar).child(main)
                }
            })
            .children(Root::render_dialog_layer(window, cx))
    }
}
