//! Style > Effects: Photoshop's classic layer styles. A switch adds or
//! removes each style; its colors, sliders, and choices edit it in place.
//! Every change is one `SetLayerStyle` command, as `spectrum canvas effect`
//! and `spectrum canvas shadow`.
use crate::{
    color_picker::{ColorPicker, Picked, color_well},
    controls::{chip, in_sidebar, slider_row, toggle},
    gradient_editor::GradientTarget,
    theme::*,
    workspace::{Workspace, slider},
};
use gpui::{prelude::*, *};
use gpui_component::slider::{SliderEvent, SliderState};
use prism_core::{
    BevelEmboss, BevelStyle, ColorOverlay, DropShadow, Glow, GradientOverlay, GradientStop,
    LayerStroke, LayerStyle, Satin, ShapeGradient, StrokePosition,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Shadow,
    Stroke,
    OuterGlow,
    InnerShadow,
    InnerGlow,
    Satin,
    ColorOverlay,
    GradientOverlay,
    Bevel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Param {
    Distance,
    Size,
    Spread,
    Angle,
    Depth,
    Altitude,
}

/// A slider: what it sets, its label, and its range and default.
type ParamSpec = (Param, &'static str, f32, f32, f32);

/// One change to a style from its controls.
#[derive(Clone, Copy)]
pub enum Change {
    Toggle(bool),
    Color([u8; 4]),
    ShadowColor([u8; 4]),
    Position(StrokePosition),
    Bevel(BevelStyle),
    Up(bool),
    Invert(bool),
    Sliders,
}

impl Effect {
    pub const ALL: [Effect; 9] = [
        Effect::Shadow,
        Effect::Stroke,
        Effect::OuterGlow,
        Effect::InnerShadow,
        Effect::InnerGlow,
        Effect::Bevel,
        Effect::Satin,
        Effect::ColorOverlay,
        Effect::GradientOverlay,
    ];

    fn label(self) -> &'static str {
        match self {
            Effect::Shadow => "Drop shadow",
            Effect::Stroke => "Stroke",
            Effect::OuterGlow => "Outer glow",
            Effect::InnerShadow => "Inner shadow",
            Effect::InnerGlow => "Inner glow",
            Effect::Satin => "Satin",
            Effect::ColorOverlay => "Color overlay",
            Effect::GradientOverlay => "Gradient overlay",
            Effect::Bevel => "Bevel & emboss",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Effect::Shadow => "effect-shadow",
            Effect::Stroke => "effect-stroke",
            Effect::OuterGlow => "effect-outer-glow",
            Effect::InnerShadow => "effect-inner-shadow",
            Effect::InnerGlow => "effect-inner-glow",
            Effect::Satin => "effect-satin",
            Effect::ColorOverlay => "effect-overlay",
            Effect::GradientOverlay => "effect-gradient-overlay",
            Effect::Bevel => "effect-bevel",
        }
    }

    fn params(self) -> &'static [ParamSpec] {
        match self {
            Effect::Shadow => &[
                (Param::Distance, "Distance", 0., 100., 17.),
                (Param::Size, "Blur", 0., 128., 10.),
                (Param::Angle, "Angle", 0., 360., 135.),
            ],
            Effect::Stroke => &[(Param::Size, "Size", 1., 50., 3.)],
            Effect::OuterGlow | Effect::InnerGlow => &[
                (Param::Size, "Size", 0., 100., 16.),
                (Param::Spread, "Spread", 0., 100., 0.),
            ],
            Effect::InnerShadow => &[
                (Param::Distance, "Distance", 0., 60., 17.),
                (Param::Size, "Blur", 0., 60., 10.),
                (Param::Angle, "Angle", 0., 360., 135.),
            ],
            Effect::Satin => &[
                (Param::Distance, "Distance", 0., 100., 11.),
                (Param::Size, "Size", 0., 100., 14.),
                (Param::Angle, "Angle", 0., 360., 19.),
            ],
            Effect::Bevel => &[
                (Param::Size, "Size", 0., 100., 8.),
                (Param::Depth, "Depth", 0., 1000., 100.),
                (Param::Angle, "Angle", 0., 360., 120.),
                (Param::Altitude, "Altitude", 0., 90., 30.),
            ],
            Effect::ColorOverlay | Effect::GradientOverlay => &[],
        }
    }

    /// Whether `style` has this effect, its main color, and its slider
    /// values in `params` order.
    fn read(self, style: &LayerStyle) -> (bool, [u8; 4], Vec<f32>) {
        // Offsets as a distance and the angle light comes from.
        let polar = |x: f32, y: f32| {
            let angle = y.atan2(-x).to_degrees().rem_euclid(360.);
            (x.hypot(y), angle)
        };
        match self {
            Effect::Shadow | Effect::InnerShadow => {
                let s = if self == Effect::Shadow {
                    style.drop_shadow
                } else {
                    style.inner_shadow
                };
                let d = s.unwrap_or_default();
                let (distance, angle) = polar(d.offset_x, d.offset_y);
                (s.is_some(), d.color, vec![distance, d.blur_radius, angle])
            }
            Effect::Stroke => {
                let d = style.stroke.unwrap_or_default();
                (style.stroke.is_some(), d.color, vec![d.size])
            }
            Effect::OuterGlow | Effect::InnerGlow => {
                let g = if self == Effect::OuterGlow {
                    style.outer_glow
                } else {
                    style.inner_glow
                };
                let d = g.unwrap_or_default();
                (g.is_some(), d.color, vec![d.size, d.spread * 100.])
            }
            Effect::Satin => {
                let d = style.satin.unwrap_or_default();
                (
                    style.satin.is_some(),
                    d.color,
                    vec![d.distance, d.size, d.angle],
                )
            }
            Effect::Bevel => {
                let d = style.bevel.unwrap_or_default();
                let values = vec![d.size, d.depth * 100., d.angle, d.altitude];
                (style.bevel.is_some(), d.highlight, values)
            }
            Effect::ColorOverlay => {
                let d = style.color_overlay.unwrap_or_default();
                (style.color_overlay.is_some(), d.color, vec![])
            }
            Effect::GradientOverlay => (style.gradient_overlay.is_some(), [0; 4], vec![]),
        }
    }
}

/// Color pickers and sliders for every style.
pub struct StyleControls {
    /// The style whose settings show below the list.
    pub chosen: Effect,
    pickers: Vec<Entity<ColorPicker>>,
    bevel_shadow: Entity<ColorPicker>,
    sliders: Vec<(Effect, Param, Entity<SliderState>)>,
    _subscriptions: Vec<Subscription>,
}

impl StyleControls {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let mut pickers = Vec::new();
        let mut sliders = Vec::new();
        let mut _subscriptions = Vec::new();
        for effect in Effect::ALL {
            let (_, color, _) = effect.read(&LayerStyle::default());
            let picker = ColorPicker::new(color, window, cx);
            _subscriptions.push(cx.subscribe_in(
                &picker,
                window,
                move |this, _, Picked(color), window, cx| {
                    this.set_effect(effect, Change::Color(*color), window, cx)
                },
            ));
            pickers.push(picker);
            for &(param, _, min, max, value) in effect.params() {
                let state = in_sidebar(slider(cx, min, max, 1., value));
                _subscriptions.push(cx.subscribe_in(
                    &state,
                    window,
                    move |this, _, _: &SliderEvent, window, cx| {
                        this.set_effect(effect, Change::Sliders, window, cx)
                    },
                ));
                sliders.push((effect, param, state));
            }
        }
        let bevel_shadow = ColorPicker::new(BevelEmboss::default().shadow, window, cx);
        _subscriptions.push(cx.subscribe_in(
            &bevel_shadow,
            window,
            |this, _, Picked(color), window, cx| {
                this.set_effect(Effect::Bevel, Change::ShadowColor(*color), window, cx)
            },
        ));
        Self {
            chosen: Effect::Shadow,
            pickers,
            bevel_shadow,
            sliders,
            _subscriptions,
        }
    }

    fn slider(&self, effect: Effect, param: Param) -> Option<&Entity<SliderState>> {
        self.sliders
            .iter()
            .find(|(e, p, _)| *e == effect && *p == param)
            .map(|(.., state)| state)
    }

    fn picker(&self, effect: Effect) -> &Entity<ColorPicker> {
        let index = Effect::ALL.iter().position(|e| *e == effect).unwrap_or(0);
        &self.pickers[index]
    }
}

impl Workspace {
    /// Adds, edits, or removes one style on the selected layer, keeping the
    /// others. Values not in `change` come from the style's sliders and the
    /// layer's current settings.
    pub fn set_effect(
        &mut self,
        effect: Effect,
        change: Change,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layer) = self.selected_layer() else {
            return;
        };
        let mut style = layer.style.clone();
        let (was_on, current, _) = effect.read(&style);
        let on = match change {
            Change::Toggle(on) => on,
            _ => was_on || !matches!(change, Change::Sliders),
        };
        if !was_on && !on {
            return;
        }
        if !was_on {
            // A style turned on starts from its defaults.
            for &(param, .., default) in effect.params() {
                if let Some(slider) = self.styles.slider(effect, param).cloned() {
                    slider.update(cx, |s, cx| s.set_value(default, window, cx));
                }
            }
        }
        let color = match change {
            Change::Color(color) => color,
            _ => current,
        };
        let value = |param: Param| {
            let spec = effect.params().iter().find(|(p, ..)| *p == param);
            self.styles
                .slider(effect, param)
                .map(|s| s.read(cx).value().start())
                .or(spec.map(|s| s.4))
                .unwrap_or(0.)
        };
        // An offset from a distance and the angle light comes from.
        let offset = |distance: f32, angle: f32| {
            let radians = angle.to_radians();
            (-radians.cos() * distance, radians.sin() * distance)
        };
        match effect {
            Effect::Shadow | Effect::InnerShadow => {
                let (x, y) = offset(value(Param::Distance), value(Param::Angle));
                let shadow = on.then_some(DropShadow {
                    color,
                    offset_x: x,
                    offset_y: y,
                    blur_radius: value(Param::Size),
                });
                if effect == Effect::Shadow {
                    style.drop_shadow = shadow;
                } else {
                    style.inner_shadow = shadow;
                }
            }
            Effect::Stroke => {
                let stroke = style.stroke.unwrap_or_default();
                style.stroke = on.then_some(LayerStroke {
                    size: value(Param::Size),
                    color,
                    position: match change {
                        Change::Position(position) => position,
                        _ => stroke.position,
                    },
                });
            }
            Effect::OuterGlow | Effect::InnerGlow => {
                let glow = on.then_some(Glow {
                    color,
                    size: value(Param::Size),
                    spread: value(Param::Spread) / 100.,
                });
                if effect == Effect::OuterGlow {
                    style.outer_glow = glow;
                } else {
                    style.inner_glow = glow;
                }
            }
            Effect::Satin => {
                let satin = style.satin.unwrap_or_default();
                style.satin = on.then_some(Satin {
                    color,
                    distance: value(Param::Distance),
                    size: value(Param::Size),
                    angle: value(Param::Angle),
                    invert: match change {
                        Change::Invert(invert) => invert,
                        _ => satin.invert,
                    },
                    ..satin
                });
            }
            Effect::Bevel => {
                let bevel = style.bevel.unwrap_or_default();
                style.bevel = on.then_some(BevelEmboss {
                    style: match change {
                        Change::Bevel(style) => style,
                        _ => bevel.style,
                    },
                    size: value(Param::Size),
                    depth: value(Param::Depth) / 100.,
                    angle: value(Param::Angle),
                    altitude: value(Param::Altitude),
                    up: match change {
                        Change::Up(up) => up,
                        _ => bevel.up,
                    },
                    highlight: color,
                    shadow: match change {
                        Change::ShadowColor(shadow) => shadow,
                        _ => bevel.shadow,
                    },
                });
            }
            Effect::ColorOverlay => {
                style.color_overlay = on.then_some(ColorOverlay { color });
            }
            Effect::GradientOverlay => {
                // A new overlay runs from the foreground to the background color.
                let (fore, back) = (self.colors.fore, self.colors.back);
                style.gradient_overlay = on.then(|| {
                    style.gradient_overlay.clone().unwrap_or(GradientOverlay {
                        gradient: ShapeGradient {
                            stops: vec![GradientStop::new(0., fore), GradientStop::new(1., back)],
                            ..ShapeGradient::default()
                        },
                        ..GradientOverlay::default()
                    })
                });
            }
        }
        self.on_selected(window, cx, |id| prism_core::Command::SetLayerStyle {
            id,
            style,
        });
        if matches!(change, Change::Toggle(true)) {
            self.sync_gradient_editors(window, cx);
        }
    }

    /// Moves the style controls to the selected layer's styles.
    pub fn sync_style_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(style) = self.selected_layer().map(|l| l.style.clone()) else {
            return;
        };
        for effect in Effect::ALL {
            let (on, color, values) = effect.read(&style);
            self.styles
                .picker(effect)
                .clone()
                .update(cx, |picker, cx| picker.set(color, window, cx));
            if !on {
                continue;
            }
            for (&(param, ..), value) in effect.params().iter().zip(values) {
                if let Some(slider) = self.styles.slider(effect, param).cloned() {
                    slider.update(cx, |s, cx| s.set_value(value, window, cx));
                }
            }
        }
        let shadow = style.bevel.unwrap_or_default().shadow;
        self.styles
            .bevel_shadow
            .clone()
            .update(cx, |picker, cx| picker.set(shadow, window, cx));
        self.sync_gradient_editors(window, cx);
    }

    /// Choice chips that send `change(value)` for `effect`.
    fn choices<T: Copy + PartialEq + 'static>(
        &self,
        effect: Effect,
        current: T,
        options: &[(&'static str, &'static str, T)],
        change: fn(T) -> Change,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .gap_1p5()
            .children(options.iter().map(|&(id, label, value)| {
                chip(id, label, value == current).on_click(cx.listener(
                    move |this, _, window, cx| this.set_effect(effect, change(value), window, cx),
                ))
            }))
    }

    /// Style > Effects, as Photoshop's Layer Style dialog: a fixed list of
    /// every style with its switch, and below it the chosen style's
    /// settings. Turning styles on and off never moves anything.
    pub fn effects_section(&self, cx: &mut Context<Self>) -> Div {
        let Some(style) = self.selected_layer().map(|l| l.style.clone()) else {
            return div();
        };
        let chosen = self.styles.chosen;
        let rows =
            Effect::ALL.map(|effect| {
                let (on, color, _) = effect.read(&style);
                let picked = effect == chosen;
                div()
                    .id(effect.id())
                    .h(px(32.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(picked, |el| el.bg(rgb(SELECTED)))
                    .when(!picked, |el| el.hover(|el| el.bg(rgb(HOVER))))
                    .child(toggle(("effect-switch", effect as usize), "", on).on_click(
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.set_effect(effect, Change::Toggle(!on), window, cx)
                        }),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(rgb(if on { TEXT } else { MUTED }))
                            .child(effect.label()),
                    )
                    .when(on && effect != Effect::GradientOverlay, |el| {
                        el.child(
                            div()
                                .size(px(12.))
                                .rounded_sm()
                                .border_1()
                                .border_color(rgb(BORDER))
                                .bg(rgba(u32::from_be_bytes(color))),
                        )
                    })
                    // Choosing a style shows its settings, and turns it on.
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.styles.chosen = effect;
                        if !on {
                            this.set_effect(effect, Change::Toggle(true), window, cx);
                        }
                        this.settle_next_frame();
                        cx.notify();
                    }))
            });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().flex().flex_col().gap_0p5().children(rows))
            .child(div().h(px(1.)).bg(rgb(BORDER)))
            .child(self.effect_settings(chosen, &style, cx))
    }

    /// The settings of one style, or a way to turn it on.
    fn effect_settings(&self, effect: Effect, style: &LayerStyle, cx: &mut Context<Self>) -> Div {
        let (on, color, _) = effect.read(style);
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(effect.label()),
            )
            .when(on && effect != Effect::GradientOverlay, |el| {
                el.child(color_well(effect.id(), color, self.styles.picker(effect)))
            });
        let mut body = div().flex().flex_col().gap_3().child(header);
        if !on {
            return body.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(rgb(FAINT))
                    .child("Off")
                    .child(
                        chip("effect-turn-on", "Turn on", false)
                            .flex_none()
                            .px_3()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_effect(effect, Change::Toggle(true), window, cx)
                            })),
                    ),
            );
        }
        for &(param, label, ..) in effect.params() {
            if let Some(state) = self.styles.slider(effect, param) {
                let value = state.read(cx).value().start();
                let text = match param {
                    Param::Angle | Param::Altitude => format!("{value:.0}°"),
                    Param::Spread | Param::Depth => format!("{value:.0}%"),
                    _ => format!("{value:.0}"),
                };
                body = body.child(slider_row(label, text, state));
            }
        }
        match effect {
            Effect::Stroke => {
                let current = style.stroke.unwrap_or_default().position;
                body = body.child(self.choices(
                    effect,
                    current,
                    &[
                        ("stroke-outside", "Outside", StrokePosition::Outside),
                        ("stroke-inside", "Inside", StrokePosition::Inside),
                        ("stroke-center", "Center", StrokePosition::Center),
                    ],
                    Change::Position,
                    cx,
                ));
            }
            Effect::Bevel => {
                let bevel = style.bevel.unwrap_or_default();
                body = body
                    .child(self.choices(
                        effect,
                        bevel.style,
                        &[
                            ("bevel-inner", "Inner", BevelStyle::Inner),
                            ("bevel-outer", "Outer", BevelStyle::Outer),
                            ("bevel-emboss", "Emboss", BevelStyle::Emboss),
                            ("bevel-pillow", "Pillow", BevelStyle::Pillow),
                        ],
                        Change::Bevel,
                        cx,
                    ))
                    .child(self.choices(
                        effect,
                        bevel.up,
                        &[("bevel-up", "Up", true), ("bevel-down", "Down", false)],
                        Change::Up,
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().text_color(rgb(MUTED)).child("Shadow"))
                            .child(color_well(
                                "bevel-shadow",
                                bevel.shadow,
                                &self.styles.bevel_shadow,
                            )),
                    );
            }
            Effect::Satin => {
                let invert = style.satin.unwrap_or_default().invert;
                body = body.child(self.choices(
                    effect,
                    invert,
                    &[
                        ("satin-edges", "Edges", true),
                        ("satin-center", "Center", false),
                    ],
                    Change::Invert,
                    cx,
                ));
            }
            Effect::GradientOverlay => {
                body = body.child(self.gradient_editor(GradientTarget::Overlay, cx));
            }
            _ => {}
        }
        body.child(
            div()
                .text_xs()
                .text_color(rgb(FAINT))
                .child("A color's alpha sets how strong its style is."),
        )
    }
}
