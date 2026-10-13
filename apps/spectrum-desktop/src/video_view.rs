//! The video editor's main area: the program monitor above, the transport,
//! and the timeline below.
use crate::{theme::*, workspace::Workspace};
use gpui::{prelude::*, *};

impl Workspace {
    pub fn video_main(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(transport) = self.transport(false, window, cx) else {
            return div().into_any_element();
        };
        let Some(video) = &self.video else {
            return div().into_any_element();
        };
        let picture = video.monitor.shown.as_ref().map(|(image, _)| {
            img(image.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
        });
        let empty = video.doc.duration().0 == 0;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .bg(rgb(0x000000))
                    .p_2()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(empty, |el| {
                        el.text_sm()
                            .text_color(rgb(FAINT))
                            .child("Add footage, audio, images, or canvases from Media (or drop files here).")
                    })
                    .when(!empty, |el| el.children(picture)),
            )
            .child(transport)
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(self.timeline(cx)),
            )
            .into_any_element()
    }
}
