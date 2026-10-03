//! Style > Effects: Photoshop's classic layer styles. A switch adds or
//! removes each style; its color, size, and position edit it in place, and
//! every change is one `SetLayerStyle` command, as `spectrum canvas effect`.
use crate::{
    color_picker::{ColorPicker, Picked, color_well},
    controls::{chip, in_sidebar, slider_row},
    theme::*,
    workspace::{Workspace, slider},
};
use gpui::{prelude::*, *};
use gpui_component::{
    slider::{SliderEvent, SliderState},
    switch::Switch,
};
use prism_core::{ColorOverlay, DropShadow, Glow, LayerStroke, LayerStyle, StrokePosition};

#[derive(Clone, Copy, PartialEq)]
pub enum Effect {
    Shadow,
    Stroke,
    OuterGlow,
    InnerShadow,
    InnerGlow,
    Overlay,
}

impl Effect {
    pub const ALL: [Effect; 6] = [
        Effect::Shadow,
        Effect::Stroke,
        Effect::OuterGlow,
        Effect::InnerShadow,
        Effect::InnerGlow,
        Effect::Overlay,
    ];

    fn label(self) -> &'static str {
        match self {
            Effect::Shadow => "Drop shadow",
            Effect::Stroke => "Stroke",
            Effect::OuterGlow => "Outer glow",
            Effect::InnerShadow => "Inner shadow",
            Effect::InnerGlow => "Inner glow",
            Effect::Overlay => "Color overlay",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Effect::Shadow => "effect-shadow",
            Effect::Stroke => "effect-stroke",
            Effect::OuterGlow => "effect-outer-glow",
            Effect::InnerShadow => "effect-inner-shadow",
            Effect::InnerGlow => "effect-inner-glow",
            Effect::Overlay => "effect-overlay",
        }
    }

    /// The style's color and size on `style`, or its defaults.
    fn settings(self, style: &LayerStyle) -> (bool, [u8; 4], f32) {
        let glow = |g: Option<Glow>| {
            let on = g.is_some();
            let g = g.unwrap_or_default();
            (on, g.color, g.size)
        };
        match self {
            Effect::Shadow => {
                let s = style.drop_shadow;
                let d = s.unwrap_or_default();
                (s.is_some(), d.color, d.blur_radius)
            }
            Effect::Stroke => {
                let s = style.stroke;
                let d = s.unwrap_or_default();
                (s.is_some(), d.color, d.size)
            }
            Effect::OuterGlow => glow(style.outer_glow),
            Effect::InnerGlow => glow(style.inner_glow),
            Effect::InnerShadow => {
                let s = style.inner_shadow;
                let d = s.unwrap_or_default();
                (s.is_some(), d.color, d.blur_radius)
            }
            Effect::Overlay => {
                let s = style.color_overlay;
                (s.is_some(), s.unwrap_or_default().color, 0.)
            }
        }
    }

    /// The size slider's range, if this style has one.
    fn range(self) -> Option<(f32, f32, f32)> {
        match self {
            Effect::Stroke => Some((1., 40., 3.)),
            Effect::OuterGlow | Effect::InnerGlow => Some((0., 100., 16.)),
            Effect::InnerShadow => Some((0., 60., 10.)),
            Effect::Shadow | Effect::Overlay => None,
        }
    }
}

/// One color picker per style, and size sliders for those that have one.
pub struct StyleControls {
    pickers: Vec<Entity<ColorPicker>>,
    sizes: Vec<Option<Entity<SliderState>>>,
    _subscriptions: Vec<Subscription>,
}

impl StyleControls {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let mut pickers = Vec::new();
        let mut sizes = Vec::new();
        let mut _subscriptions = Vec::new();
        for effect in Effect::ALL {
            let (_, color, _) = effect.settings(&LayerStyle::default());
            let picker = ColorPicker::new(color, window, cx);
            _subscriptions.push(cx.subscribe_in(
                &picker,
                window,
                move |this, _, Picked(color), window, cx| {
                    this.set_effect(effect, true, Some(*color), None, window, cx)
                },
            ));
            pickers.push(picker);
            let size = effect.range().map(|(min, max, value)| {
                let state = in_sidebar(slider(cx, min, max, 1., value));
                _subscriptions.push(cx.subscribe_in(
                    &state,
                    window,
                    move |this, _, _: &SliderEvent, window, cx| {
                        this.set_effect(effect, true, None, None, window, cx)
                    },
                ));
                state
            });
            sizes.push(size);
        }
        Self {
            pickers,
            sizes,
            _subscriptions,
        }
    }
}

impl Workspace {
    /// Adds, edits, or removes one style on the selected layer, keeping the
    /// others. Unset parts keep the layer's current values.
    pub fn set_effect(
        &mut self,
        effect: Effect,
        on: bool,
        color: Option<[u8; 4]>,
        position: Option<StrokePosition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layer) = self.selected_layer() else {
            return;
        };
        let mut style = layer.style;
        let (_, current, _) = effect.settings(&style);
        let color = color.unwrap_or(current);
        let size = self.styles.sizes[effect as usize]
            .as_ref()
            .map_or(0., |s| s.read(cx).value().start());
        let distance = self.shadow_distance.read(cx).value().start();
        let blur = self.shadow_blur.read(cx).value().start();
        let glow = |g: Option<Glow>| Glow {
            color,
            size,
            ..g.unwrap_or_default()
        };
        match effect {
            Effect::Shadow => {
                style.drop_shadow = on.then_some(DropShadow {
                    color,
                    offset_x: distance,
                    offset_y: distance,
                    blur_radius: blur,
                })
            }
            Effect::Stroke => {
                let stroke = style.stroke.unwrap_or_default();
                style.stroke = on.then_some(LayerStroke {
                    size,
                    color,
                    position: position.unwrap_or(stroke.position),
                });
            }
            Effect::OuterGlow => style.outer_glow = on.then(|| glow(style.outer_glow)),
            Effect::InnerGlow => style.inner_glow = on.then(|| glow(style.inner_glow)),
            Effect::InnerShadow => {
                style.inner_shadow = on.then_some(DropShadow {
                    color,
                    offset_x: size / 2.,
                    offset_y: size / 2.,
                    blur_radius: size,
                })
            }
            Effect::Overlay => style.color_overlay = on.then_some(ColorOverlay { color }),
        }
        self.on_selected(window, cx, |id| prism_core::Command::SetLayerStyle {
            id,
            style,
        });
    }

    /// Moves the style controls to the selected layer's styles.
    pub fn sync_style_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(style) = self.selected_layer().map(|l| l.style) else {
            return;
        };
        for effect in Effect::ALL {
            let (on, color, size) = effect.settings(&style);
            self.styles.pickers[effect as usize]
                .update(cx, |picker, cx| picker.set(color, window, cx));
            if let Some(slider) = &self.styles.sizes[effect as usize]
                && on
            {
                slider.update(cx, |s, cx| s.set_value(size, window, cx));
            }
        }
    }

    /// Style > Effects: a switch per style, and its settings while on.
    pub fn effects_section(&self, cx: &mut Context<Self>) -> Div {
        let Some(style) = self.selected_layer().map(|l| l.style) else {
            return div();
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .children(Effect::ALL.map(|effect| {
                let (on, color, _) = effect.settings(&style);
                let header = div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        Switch::new(effect.id())
                            .label(effect.label())
                            .checked(on)
                            .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                                this.set_effect(effect, *checked, None, None, window, cx)
                            })),
                    )
                    .when(on, |el| {
                        el.child(color_well(
                            effect.id(),
                            color,
                            &self.styles.pickers[effect as usize],
                        ))
                    });
                let mut body = div().flex().flex_col().gap_3().child(header);
                if !on {
                    return body;
                }
                if effect == Effect::Shadow {
                    let distance = self.shadow_distance.read(cx).value().start();
                    let blur = self.shadow_blur.read(cx).value().start();
                    body = body
                        .child(slider_row(
                            "Distance",
                            format!("{distance:.0}"),
                            &self.shadow_distance,
                        ))
                        .child(slider_row("Blur", format!("{blur:.0}"), &self.shadow_blur));
                }
                if let Some(size) = &self.styles.sizes[effect as usize] {
                    let value = size.read(cx).value().start();
                    body = body.child(slider_row("Size", format!("{value:.0}"), size));
                }
                if effect == Effect::Stroke {
                    let current = style.stroke.unwrap_or_default().position;
                    body = body.child(
                        div().flex().gap_1p5().children(
                            [
                                ("stroke-outside", "Outside", StrokePosition::Outside),
                                ("stroke-inside", "Inside", StrokePosition::Inside),
                                ("stroke-center", "Center", StrokePosition::Center),
                            ]
                            .map(|(id, label, position)| {
                                chip(id, label, position == current).on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.set_effect(
                                            Effect::Stroke,
                                            true,
                                            None,
                                            Some(position),
                                            window,
                                            cx,
                                        )
                                    },
                                ))
                            }),
                        ),
                    );
                }
                body
            }))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(FAINT))
                    .child("A color's alpha sets how strong its style is."),
            )
    }
}
