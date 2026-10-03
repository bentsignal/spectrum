//! Style mode: one section at a time for the selected layer's look, its
//! arrangement, and its shadow; with nothing selected, the canvas itself.
use crate::{
    controls::{chip, group, slider_row},
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::{Sizable, button::Button};
use prism_core::{Alignment, AlignmentReference, Command};

pub const SECTIONS: [&str; 3] = ["Look", "Arrange", "Effects"];
const ALIGN: [(&str, Alignment); 6] = [
    ("Left", Alignment::Left),
    ("Center", Alignment::HorizontalCenter),
    ("Right", Alignment::Right),
    ("Top", Alignment::Top),
    ("Middle", Alignment::VerticalCenter),
    ("Bottom", Alignment::Bottom),
];

impl Workspace {
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
            .child(self.guides_section(cx))
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
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                group("Align to canvas", None)
                    .gap_4()
                    .child(div().grid().grid_cols(3).gap_1p5().children(chips))
                    .child(slider_row(
                        "Rotation",
                        format!("{rotation:.0}°"),
                        &self.rotation,
                    )),
            )
            .child(self.guides_section(cx))
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
            2 => self.effects_section(cx),
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
