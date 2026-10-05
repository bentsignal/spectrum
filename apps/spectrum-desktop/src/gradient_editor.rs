//! A gradient editor for shape fills and gradient overlays: a preview bar
//! whose stops drag along it, the selected stop's color, stops added and
//! removed, and the kind and angle. Every change is one engine command
//! (`SetShapeFill` or `SetLayerStyle`), as `spectrum canvas gradient` and
//! `spectrum canvas effect <layer> gradient-overlay`.
use crate::{
    color_picker::{ColorPicker, Picked, color_well},
    controls::{chip, in_sidebar, slider_row},
    theme::*,
    workspace::{Workspace, slider},
};
use gpui::{prelude::*, *};
use gpui_component::slider::{SliderEvent, SliderState};
use spectrum_canvas::{
    Command, GradientKind, GradientOverlay, GradientStop, ShapeFill, ShapeGradient,
};
use std::{cell::Cell, rc::Rc};

/// Writes a slider's value into one field of a gradient.
type SetField = fn(&mut ShapeGradient, f32);

/// What a gradient editor edits.
#[derive(Clone, Copy, PartialEq)]
pub enum GradientTarget {
    Fill,
    Overlay,
}

pub struct GradientEditor {
    picker: Entity<ColorPicker>,
    angle: Entity<SliderState>,
    /// Center (percent of the layer), radial size, and linear scale.
    center_x: Entity<SliderState>,
    center_y: Entity<SliderState>,
    size: Entity<SliderState>,
    scale: Entity<SliderState>,
    selected: usize,
    dragging: Option<usize>,
    bar: Rc<Cell<Bounds<Pixels>>>,
    _subscriptions: Vec<Subscription>,
}

impl GradientEditor {
    pub fn new(target: GradientTarget, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let picker = ColorPicker::new([255, 255, 255, 255], window, cx);
        let angle = in_sidebar(slider(cx, 0., 360., 1., 0.));
        let center_x = in_sidebar(slider(cx, 0., 100., 1., 50.));
        let center_y = in_sidebar(slider(cx, 0., 100., 1., 50.));
        let size = in_sidebar(slider(cx, 5., 300., 1., 50.));
        let scale = in_sidebar(slider(cx, 10., 400., 1., 100.));
        let mut _subscriptions = vec![
            cx.subscribe_in(
                &picker,
                window,
                move |this, _, Picked(color), window, cx| {
                    let color = *color;
                    this.edit_gradient(target, window, cx, |gradient, selected| {
                        if let Some(stop) = gradient.stops.get_mut(selected) {
                            stop.color = color;
                        }
                    })
                },
            ),
            cx.subscribe_in(
                &angle,
                window,
                move |this, state, _: &SliderEvent, window, cx| {
                    let angle = state.read(cx).value().start();
                    this.edit_gradient(target, window, cx, |gradient, _| gradient.angle = angle)
                },
            ),
        ];
        // Each shape slider writes one field of the gradient.
        let fields: [(&Entity<SliderState>, SetField); 4] = [
            (&center_x, |g, v| g.center[0] = v / 100.),
            (&center_y, |g, v| g.center[1] = v / 100.),
            (&size, |g, v| g.radius = (v / 100.).max(0.01)),
            (&scale, |g, v| g.extent = (v / 100.).max(0.01)),
        ];
        for (state, set) in fields {
            _subscriptions.push(cx.subscribe_in(
                state,
                window,
                move |this, state, _: &SliderEvent, window, cx| {
                    let value = state.read(cx).value().start();
                    this.edit_gradient(target, window, cx, |gradient, _| set(gradient, value))
                },
            ));
        }
        Self {
            picker,
            angle,
            center_x,
            center_y,
            size,
            scale,
            selected: 0,
            dragging: None,
            bar: Rc::default(),
            _subscriptions,
        }
    }

    fn editor_mut(this: &mut Workspace, target: GradientTarget) -> &mut GradientEditor {
        match target {
            GradientTarget::Fill => &mut this.fill_gradient,
            GradientTarget::Overlay => &mut this.overlay_gradient,
        }
    }
}

/// The gradient's stops sorted, as the engine keeps them.
fn sorted(mut gradient: ShapeGradient) -> ShapeGradient {
    gradient
        .stops
        .sort_by(|a, b| a.position.total_cmp(&b.position));
    gradient
}

fn hsla_of([r, g, b, a]: [u8; 4]) -> Hsla {
    rgba(u32::from_be_bytes([r, g, b, a])).into()
}

impl Workspace {
    /// The selected layer's gradient for `target`, if it has one.
    pub fn current_gradient(&self, target: GradientTarget) -> Option<ShapeGradient> {
        let layer = self.selected_layer()?;
        match target {
            GradientTarget::Fill => layer
                .shape_fill
                .as_ref()
                .map(|ShapeFill::Gradient(gradient)| gradient.clone()),
            GradientTarget::Overlay => layer
                .style
                .gradient_overlay
                .as_ref()
                .map(|o| o.gradient.clone()),
        }
    }

    /// Applies `gradient` to the selected layer as a fill or an overlay.
    pub fn set_gradient(
        &mut self,
        target: GradientTarget,
        gradient: Option<ShapeGradient>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layer) = self.selected_layer() else {
            return;
        };
        let gradient = gradient.map(sorted);
        let command = match target {
            GradientTarget::Fill => Command::SetShapeFill {
                id: layer.id,
                fill: gradient.map(ShapeFill::Gradient),
            },
            GradientTarget::Overlay => {
                let mut style = layer.style.clone();
                let blend_mode = style
                    .gradient_overlay
                    .as_ref()
                    .map_or(spectrum_canvas::BlendMode::Normal, |o| o.blend_mode);
                style.gradient_overlay = gradient.map(|gradient| GradientOverlay {
                    gradient,
                    blend_mode,
                });
                Command::SetLayerStyle {
                    id: layer.id,
                    style,
                }
            }
        };
        self.canvas_commands(vec![command], window, cx);
    }

    /// Turns the selected shape's fill into a gradient from its color to
    /// the background color, unless it is one already.
    pub fn start_fill_gradient(
        &mut self,
        color: [u8; 4],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.current_gradient(GradientTarget::Fill).is_some() {
            return;
        }
        let gradient = ShapeGradient {
            stops: vec![
                GradientStop::new(0., color),
                GradientStop::new(1., self.colors.back),
            ],
            ..ShapeGradient::default()
        };
        self.fill_gradient.selected = 0;
        self.set_gradient(GradientTarget::Fill, Some(gradient), window, cx);
        self.sync_gradient_editors(window, cx);
    }

    /// Edits the current gradient in place; `selected` is the chosen stop.
    fn edit_gradient(
        &mut self,
        target: GradientTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut ShapeGradient, usize),
    ) {
        let Some(mut gradient) = self.current_gradient(target) else {
            return;
        };
        let selected = GradientEditor::editor_mut(self, target).selected;
        edit(&mut gradient, selected);
        self.set_gradient(target, Some(gradient), window, cx);
    }

    /// Shows the selected layer's gradient in the editor's controls.
    pub fn sync_gradient_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for target in [GradientTarget::Fill, GradientTarget::Overlay] {
            let gradient = self.current_gradient(target);
            let editor = GradientEditor::editor_mut(self, target);
            let Some(gradient) = gradient else {
                editor.selected = 0;
                continue;
            };
            editor.selected = editor.selected.min(gradient.stops.len().saturating_sub(1));
            let color = gradient.stops[editor.selected].color;
            let values = [
                (editor.angle.clone(), gradient.angle),
                (editor.center_x.clone(), gradient.center[0] * 100.),
                (editor.center_y.clone(), gradient.center[1] * 100.),
                (editor.size.clone(), gradient.radius * 100.),
                (editor.scale.clone(), gradient.extent * 100.),
            ];
            let picker = editor.picker.clone();
            picker.update(cx, |picker, cx| picker.set(color, window, cx));
            for (slider, value) in values {
                slider.update(cx, |s, cx| s.set_value(value, window, cx));
            }
        }
    }

    fn select_stop(
        &mut self,
        target: GradientTarget,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        GradientEditor::editor_mut(self, target).selected = index;
        self.sync_gradient_editors(window, cx);
        cx.notify();
    }

    /// Adds a stop halfway between the selected stop and the next, in the
    /// color the gradient already has there.
    fn add_stop(&mut self, target: GradientTarget, window: &mut Window, cx: &mut Context<Self>) {
        let Some(gradient) = self.current_gradient(target) else {
            return;
        };
        let selected = GradientEditor::editor_mut(self, target).selected;
        let here = gradient.stops[selected].position;
        let next = gradient
            .stops
            .get(selected + 1)
            .map_or(1.0, |stop| stop.position);
        let position = if next - here > 0.02 {
            (here + next) / 2.
        } else {
            (here / 2.).max(0.)
        };
        let color = gradient.sampler().sample_position(position);
        let mut gradient = gradient;
        gradient.stops.push(GradientStop::new(position, color));
        let gradient = sorted(gradient);
        let index = gradient
            .stops
            .iter()
            .position(|stop| stop.position == position)
            .unwrap_or(0);
        GradientEditor::editor_mut(self, target).selected = index;
        self.set_gradient(target, Some(gradient), window, cx);
        self.sync_gradient_editors(window, cx);
    }

    fn remove_stop(&mut self, target: GradientTarget, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut gradient) = self.current_gradient(target) else {
            return;
        };
        if gradient.stops.len() <= 2 {
            return;
        }
        let selected = GradientEditor::editor_mut(self, target).selected;
        gradient
            .stops
            .remove(selected.min(gradient.stops.len() - 1));
        GradientEditor::editor_mut(self, target).selected = selected.saturating_sub(1);
        self.set_gradient(target, Some(gradient), window, cx);
        self.sync_gradient_editors(window, cx);
    }

    /// Moves the dragged stop to where the pointer is along the bar.
    fn drag_stop(
        &mut self,
        target: GradientTarget,
        x: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = GradientEditor::editor_mut(self, target);
        let Some(index) = editor.dragging else {
            return;
        };
        let bar = editor.bar.get();
        let position =
            (f32::from(x - bar.origin.x) / f32::from(bar.size.width).max(1.)).clamp(0., 1.);
        let Some(mut gradient) = self.current_gradient(target) else {
            return;
        };
        let Some(stop) = gradient.stops.get_mut(index) else {
            return;
        };
        let color = stop.color;
        stop.position = position;
        let gradient = sorted(gradient);
        // The dragged stop keeps its selection as it passes others.
        let index = gradient
            .stops
            .iter()
            .position(|stop| stop.position == position && stop.color == color)
            .unwrap_or(index);
        let editor = GradientEditor::editor_mut(self, target);
        editor.dragging = Some(index);
        editor.selected = index;
        self.set_gradient(target, Some(gradient), window, cx);
    }

    /// The editor for the selected layer's gradient `target`.
    pub fn gradient_editor(&self, target: GradientTarget, cx: &mut Context<Self>) -> Div {
        let editor = match target {
            GradientTarget::Fill => &self.fill_gradient,
            GradientTarget::Overlay => &self.overlay_gradient,
        };
        let Some(gradient) = self.current_gradient(target) else {
            return div();
        };
        let stops = gradient.stops.clone();
        let selected = editor.selected.min(stops.len().saturating_sub(1));
        let bar_slot = editor.bar.clone();
        let segments = stops.windows(2).map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(a.position))
                .w(relative((b.position - a.position).max(0.)))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(hsla_of(a.color), 0.),
                    linear_color_stop(hsla_of(b.color), 1.),
                ))
        });
        let first = stops.first().map_or([0; 4], |s| s.color);
        let last = stops.last().map_or([0; 4], |s| s.color);
        let ends = [
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(relative(stops.first().map_or(0., |s| s.position)))
                .bg(hsla_of(first)),
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right_0()
                .w(relative(1. - stops.last().map_or(1., |s| s.position)))
                .bg(hsla_of(last)),
        ];
        let markers = stops.iter().enumerate().map(|(index, stop)| {
            let on = index == selected;
            div()
                .id(("gradient-stop", index))
                .absolute()
                .top(px(26.))
                .left(relative(stop.position))
                .ml(px(-7.))
                .size(px(14.))
                .rounded_sm()
                .border_2()
                .border_color(rgb(if on { 0xf4f4f4 } else { 0x5a5a5a }))
                .bg(hsla_of(stop.color))
                .cursor_ew_resize()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        GradientEditor::editor_mut(this, target).dragging = Some(index);
                        this.select_stop(target, index, window, cx);
                        cx.stop_propagation();
                    }),
                )
        });
        let kinds = [
            ("Linear", GradientKind::Linear),
            ("Radial", GradientKind::Radial),
            ("Angle", GradientKind::Angle),
        ]
        .map(|(label, kind)| {
            chip(label, label, gradient.kind == kind).on_click(cx.listener(
                move |this, _, window, cx| {
                    this.edit_gradient(target, window, cx, |gradient, _| gradient.kind = kind)
                },
            ))
        });
        let angle = editor.angle.read(cx).value().start();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .id(match target {
                        GradientTarget::Fill => "fill-gradient-bar",
                        GradientTarget::Overlay => "overlay-gradient-bar",
                    })
                    .relative()
                    .h(px(44.))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(px(22.))
                            .rounded_sm()
                            .overflow_hidden()
                            .border_1()
                            .border_color(rgb(BORDER))
                            .children(ends)
                            .children(segments)
                            .child(
                                canvas(move |bounds, _, _| bar_slot.set(bounds), |_, _, _, _| {})
                                    .absolute()
                                    .size_full(),
                            ),
                    )
                    .children(markers)
                    .on_mouse_move(
                        cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                            if event.pressed_button == Some(MouseButton::Left) {
                                this.drag_stop(target, event.position.x, window, cx);
                            }
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, _| {
                            GradientEditor::editor_mut(this, target).dragging = None;
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().text_color(rgb(MUTED)).child("Stop"))
                            .child(color_well(
                                match target {
                                    GradientTarget::Fill => "fill-stop-color",
                                    GradientTarget::Overlay => "overlay-stop-color",
                                },
                                stops[selected].color,
                                &editor.picker,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1p5()
                            .child(
                                chip("add-stop", "Add", false)
                                    .px_2p5()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.add_stop(target, window, cx)
                                    })),
                            )
                            .when(stops.len() > 2, |el| {
                                el.child(chip("remove-stop", "Remove", false).px_2p5().on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        this.remove_stop(target, window, cx)
                                    }),
                                ))
                            })
                            .child(chip("reverse-stops", "Reverse", false).px_2p5().on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.edit_gradient(target, window, cx, |gradient, _| {
                                        for stop in &mut gradient.stops {
                                            stop.position = 1. - stop.position;
                                        }
                                    })
                                }),
                            )),
                    ),
            )
            .child(div().flex().gap_1p5().children(kinds))
            .when(gradient.kind != GradientKind::Radial, |el| {
                el.child(slider_row("Angle", format!("{angle:.0}°"), &editor.angle))
            })
            .map(|el| {
                let percent =
                    |state: &Entity<SliderState>| format!("{:.0}%", state.read(cx).value().start());
                match gradient.kind {
                    GradientKind::Linear => {
                        el.child(slider_row("Scale", percent(&editor.scale), &editor.scale))
                    }
                    kind => el
                        .child(slider_row(
                            "Center X",
                            percent(&editor.center_x),
                            &editor.center_x,
                        ))
                        .child(slider_row(
                            "Center Y",
                            percent(&editor.center_y),
                            &editor.center_y,
                        ))
                        .when(kind == GradientKind::Radial, |el| {
                            el.child(slider_row("Size", percent(&editor.size), &editor.size))
                        }),
                }
            })
    }
}
