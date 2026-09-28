//! Style mode: one section at a time for the selected layer's look, its
//! arrangement, and its shadow; with nothing selected, the canvas itself.
use crate::{
    controls::{chip, group, slider_row},
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::{Sizable, button::Button, switch::Switch};
use prism_core::{Alignment, AlignmentReference, Command, DropShadow, LayerStyle};

pub const SECTIONS: [&str; 3] = ["Look", "Arrange", "Shadow"];
const ALIGN: [(&str, Alignment); 6] = [
    ("Left", Alignment::Left),
    ("Center", Alignment::HorizontalCenter),
    ("Right", Alignment::Right),
    ("Top", Alignment::Top),
    ("Middle", Alignment::VerticalCenter),
    ("Bottom", Alignment::Bottom),
];

impl Workspace {
    /// Applies the shadow sliders, or removes the shadow when `on` is false.
    pub fn set_shadow(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let distance = self.shadow_distance.read(cx).value().start();
        let blur = self.shadow_blur.read(cx).value().start();
        let style = LayerStyle {
            drop_shadow: on.then(|| DropShadow {
                offset_x: distance,
                offset_y: distance,
                blur_radius: blur,
                ..DropShadow::default()
            }),
        };
        self.on_selected(window, cx, |id| Command::SetLayerStyle { id, style });
    }

    fn canvas_settings(&self, cx: &mut Context<Self>) -> Div {
        let look = self.look_section(cx);
        let (w, h) = self
            .canvas
            .as_ref()
            .map_or((0, 0), |c| (c.doc.width, c.doc.height));
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                group("Canvas size", None).child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_sm().child(format!("{w} × {h} px")))
                        .child(
                            Button::new("resize-canvas")
                                .small()
                                .label("Resize…")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_canvas_size(window, cx)
                                })),
                        ),
                ),
            )
            .child(look)
    }

    fn arrange_section(&self, cx: &mut Context<Self>) -> Div {
        let chips = ALIGN.iter().map(|(label, alignment)| {
            let alignment = *alignment;
            chip(label, label, false).on_click(cx.listener(move |this, _, window, cx| {
                this.on_selected(window, cx, |id| Command::AlignLayer {
                    id,
                    alignment,
                    reference: AlignmentReference::Canvas,
                })
            }))
        });
        let rotation = self.rotation.read(cx).value().start();
        group("Align to canvas", None)
            .gap_4()
            .child(div().grid().grid_cols(3).gap_1p5().children(chips))
            .child(slider_row(
                "Rotation",
                format!("{rotation:.0}°"),
                &self.rotation,
            ))
    }

    fn shadow_section(&self, cx: &mut Context<Self>) -> Div {
        let on = self
            .selected_layer()
            .is_some_and(|l| l.style.drop_shadow.is_some());
        let distance = self.shadow_distance.read(cx).value().start();
        let blur = self.shadow_blur.read(cx).value().start();
        group("Drop shadow", None)
            .gap_4()
            .child(
                Switch::new("shadow")
                    .label("Show a shadow")
                    .checked(on)
                    .on_click(cx.listener(|this, checked, window, cx| {
                        this.set_shadow(*checked, window, cx)
                    })),
            )
            .when(on, |el| {
                el.child(slider_row(
                    "Distance",
                    format!("{distance:.0}"),
                    &self.shadow_distance,
                ))
                .child(slider_row(
                    "Blur",
                    format!("{blur:.0}"),
                    &self.shadow_blur,
                ))
            })
    }

    pub fn style_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.selected_layer().is_none() {
            return self.canvas_settings(cx);
        }
        let chips: Vec<_> = SECTIONS
            .iter()
            .enumerate()
            .map(|(index, name)| {
                chip(name, name, index == self.style_section)
                    .flex_none()
                    .px_2p5()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.style_section = index;
                        this.settle_next_frame();
                        cx.notify();
                    }))
            })
            .collect();
        let body = match self.style_section {
            1 => self.arrange_section(cx),
            2 => self.shadow_section(cx),
            _ => self.look_section(cx),
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(div().flex().flex_wrap().gap_1p5().children(chips))
            .child(body)
    }
}
