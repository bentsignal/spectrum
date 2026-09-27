//! The project overview's sidebar: import into the project, search, filters.
use crate::{
    theme::*,
    workspace::{Open, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{Icon, IconName, Sizable, input::Input};

impl Workspace {
    pub fn project_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let import = self.import_button(cx);
        let chips = self.kind_chips(cx).into_any_element();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(import)
            .child(
                Input::new(&self.search)
                    .prefix(Icon::new(IconName::Search).small().text_color(rgb(MUTED)))
                    .cleanable(true),
            )
            .child(chips)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xs().text_color(rgb(MUTED)).child("Sample"))
                    .child(
                        div()
                            .id("sample")
                            .h(px(36.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .rounded_md()
                            .text_sm()
                            .hover(|el| el.bg(rgb(HOVER)))
                            .child(Icon::new(IconName::Frame).small().text_color(rgb(MUTED)))
                            .child("Spring poster canvas")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_item(Open::Sample, window, cx)
                            })),
                    ),
            )
    }
}
