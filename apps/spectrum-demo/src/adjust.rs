use crate::{
    controls::{group, slider_row},
    samples::{self, Look},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    slider::SliderState,
    switch::Switch,
};

/// Signed slider readout that shows zero without a sign.
fn signed(value: f32, decimals: usize) -> String {
    if value.abs() < 0.005 {
        return "0".into();
    }
    let text = format!("{:+.*}", decimals, value);
    text.replacen('-', "−", 1)
}

fn reset(
    id: &'static str,
    states: [Entity<SliderState>; 2],
    cx: &mut Context<Workspace>,
) -> AnyElement {
    Button::new(id)
        .ghost()
        .xsmall()
        .label("Reset")
        .text_color(rgb(MUTED))
        .on_click(cx.listener(move |_, _, window, cx| {
            for state in &states {
                state.update(cx, |state, cx| state.set_value(0., window, cx));
            }
            cx.notify();
        }))
        .into_any_element()
}

impl Workspace {
    pub fn adjust_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let value = |state: &Entity<SliderState>| state.read(cx).value().start();
        let exposure = value(&self.exposure);
        let contrast = value(&self.contrast);
        let temperature = value(&self.temperature);
        let saturation = value(&self.saturation);
        let reset_light = reset(
            "reset-light",
            [self.exposure.clone(), self.contrast.clone()],
            cx,
        );
        let reset_color = reset(
            "reset-color",
            [self.temperature.clone(), self.saturation.clone()],
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                group("Light", Some(reset_light))
                    .gap_4()
                    .child(slider_row("Exposure", signed(exposure, 2), &self.exposure))
                    .child(slider_row("Contrast", signed(contrast, 0), &self.contrast)),
            )
            .child(
                group("Color", Some(reset_color))
                    .gap_4()
                    .child(slider_row(
                        "Temperature",
                        signed(temperature, 0),
                        &self.temperature,
                    ))
                    .child(slider_row(
                        "Saturation",
                        signed(saturation, 0),
                        &self.saturation,
                    )),
            )
            .child(
                group("View", None).child(
                    Switch::new("compare")
                        .label("Compare with original")
                        .text_sm()
                        .checked(self.compare)
                        .on_click(cx.listener(|this, checked, _, cx| {
                            this.compare = *checked;
                            cx.notify();
                        })),
                ),
            )
    }

    pub fn adjust(&self, area: Size<f32>) -> impl IntoElement {
        let asset = &self.assets[self.selected];
        let placed = &self.assets[0];
        let aspect = samples::aspect(asset);
        let gap = 16.;
        let columns = if self.compare { 2. } else { 1. };
        let available = size(
            (area.width - 96. - gap * (columns - 1.)) / columns,
            area.height - 96. - 28.,
        );
        let width = available.width.min(available.height / aspect).max(80.);
        let image = |look: Look, label: &'static str| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .rounded_md()
                        .overflow_hidden()
                        .child(samples::artwork(asset, look, placed, width)),
                )
                .when(self.compare, |el| {
                    el.child(div().text_xs().text_color(rgb(FAINT)).child(label))
                })
        };
        div()
            .size_full()
            .pb(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(gap))
            .when(self.compare, |el| {
                el.child(image(Look::default(), "Original"))
            })
            .child(image(asset.look, "Edited"))
    }
}
