use crate::{
    controls::{Field, group, segmented, slider_row},
    samples,
    theme::*,
    workspace::{Mode, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
};

const BLENDS: [&str; 6] = [
    "Normal",
    "Multiply",
    "Screen",
    "Overlay",
    "Soft light",
    "Color",
];
/// Index of the canvas asset and of the photo layer placed in it.
const CANVAS: usize = 1;
const PHOTO_LAYER: usize = 1;

impl Workspace {
    pub fn canvas_sidebar(&self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let layer = &self.layers[self.layer];
        let blend = Field::new("blend", BLENDS[layer.blend]).options(
            BLENDS.map(SharedString::from).to_vec(),
            {
                let view = view.clone();
                move |cx| {
                    let this = view.read(cx);
                    this.layers[this.layer].blend
                }
            },
            {
                let view = view.clone();
                move |index, _, cx| {
                    view.update(cx, |this, cx| {
                        let layer = this.layer;
                        this.layers[layer].blend = index;
                        cx.notify();
                    })
                }
            },
        );
        let scope = segmented(
            "scope",
            [(None, "Edit locally"), (None, "Edit globally")],
            self.global as usize,
            move |index, _, cx| {
                view.update(cx, |this, cx| {
                    this.global = index == 1;
                    cx.notify();
                })
            },
        );
        let asset = self.assets[0].name.clone();
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                group("Layers", None)
                    .gap_1()
                    .children(self.layers.iter().enumerate().map(|(index, layer)| {
                        let selected = index == self.layer;
                        div()
                            .id(("layer", index))
                            .h(px(36.))
                            .pl_2p5()
                            .pr_1()
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .rounded_md()
                            .when(selected, |el| el.bg(rgb(SELECTED)))
                            .when(!selected, |el| el.hover(|el| el.bg(rgb(HOVER))))
                            .child(Icon::new(layer.icon.clone()).small().text_color(rgb(MUTED)))
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .truncate()
                                    .text_color(rgb(if layer.visible { TEXT } else { FAINT }))
                                    .child(layer.name),
                            )
                            .child(
                                Button::new(("visibility", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(if layer.visible {
                                        IconName::Eye
                                    } else {
                                        IconName::EyeOff
                                    })
                                    .tooltip(if layer.visible { "Hide" } else { "Show" })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.layers[index].visible = !this.layers[index].visible;
                                        cx.stop_propagation();
                                        cx.notify();
                                    })),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_layer(index, window, cx)
                            }))
                    })),
            )
            .child(
                group("Blending", None)
                    .gap_4()
                    .child(blend)
                    .child(slider_row(
                        "Opacity",
                        format!("{:.0}%", layer.opacity),
                        &self.opacity,
                    )),
            )
            .when(self.layer == PHOTO_LAYER, |el| {
                el.child(
                    group("Edits", None)
                        .child(scope)
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(MUTED))
                                .child(if self.global {
                                    format!("Changes apply to {asset} everywhere it is used.")
                                } else {
                                    "Changes apply only to this canvas.".to_string()
                                }),
                        )
                        .when(self.global, |el| {
                            el.child(
                                Button::new("adjust-asset")
                                    .label("Adjust color")
                                    .w_full()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.set_mode(Mode::Color, window, cx)
                                    })),
                            )
                        }),
                )
            })
    }

    pub fn canvas(&self, area: Size<f32>, _: &mut Context<Self>) -> impl IntoElement {
        let canvas = &self.assets[CANVAS];
        let width = (area.width - 112.)
            .min((area.height - 112.) * 16. / 9.)
            .max(160.);
        let height = width * 9. / 16.;
        let visible = [0, 1, 2].map(|index| self.layers[index].visible);
        let opacity = [0, 1, 2].map(|index| self.layers[index].opacity / 100.);
        let (left, top, w, h) = match self.layer {
            0 => (0.585, 0.33, 0.34, 0.25),
            1 => (0.07, 0.14, 0.46, 0.46 * 0.75 * 16. / 9.),
            _ => (0., 0., 1., 1.),
        };
        div()
            .size_full()
            .pb(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .relative()
                    .shadow_lg()
                    .child(samples::composition(
                        canvas,
                        canvas.look,
                        &self.assets[0],
                        width,
                        visible,
                        opacity,
                    ))
                    .child(
                        div()
                            .absolute()
                            .left(px(left * width - 4.))
                            .top(px(top * height - 4.))
                            .w(px(w * width + 8.))
                            .h(px(h * height + 8.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(hsla(0., 0., 1., 0.9)),
                    ),
            )
    }
}
