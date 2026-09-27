//! Color mode for the sample canvas: acts on the selected layer, showing that
//! color tools belong to whatever is selected rather than to one asset type.
use crate::{
    color::signed,
    controls::{group, slider_row},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    slider::SliderState,
};

impl Workspace {
    fn sample_reset(
        &self,
        id: &'static str,
        states: [Entity<SliderState>; 2],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(id)
            .ghost()
            .xsmall()
            .label("Reset")
            .text_color(rgb(MUTED))
            .on_click(cx.listener(move |this, _, window, cx| {
                for state in &states {
                    state.update(cx, |state, cx| state.set_value(0., window, cx));
                }
                this.select_layer_look_from_sliders(cx);
                cx.notify();
            }))
            .into_any_element()
    }

    fn select_layer_look_from_sliders(&mut self, cx: &mut Context<Self>) {
        let value = |state: &Entity<SliderState>| state.read(cx).value().start();
        if let Some(target) = self.sample_target() {
            let look = &mut self.assets[target].look;
            look.exposure = value(&self.exposure);
            look.contrast = value(&self.contrast);
            look.temperature = value(&self.temperature);
            look.saturation = value(&self.saturation);
        }
    }

    pub fn sample_color_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let layer = self.layers[self.layer].name;
        let heading = div()
            .text_sm()
            .text_color(rgb(MUTED))
            .child(format!("Editing {layer}"));
        if self.sample_target().is_none() {
            return div()
                .flex()
                .flex_col()
                .gap_3()
                .child(heading)
                .child(
                    div().text_sm().text_color(rgb(FAINT)).child(
                        "Color applies to image and background layers. Choose one in Layers.",
                    ),
                )
                .into_any_element();
        }
        let value = |state: &Entity<SliderState>| state.read(cx).value().start();
        let (exposure, contrast, temperature, saturation) = (
            value(&self.exposure),
            value(&self.contrast),
            value(&self.temperature),
            value(&self.saturation),
        );
        let reset_light = self.sample_reset(
            "reset-light",
            [self.exposure.clone(), self.contrast.clone()],
            cx,
        );
        let reset_color = self.sample_reset(
            "reset-color",
            [self.temperature.clone(), self.saturation.clone()],
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(heading)
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
            .into_any_element()
    }
}
