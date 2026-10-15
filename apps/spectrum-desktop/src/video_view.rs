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
        // Footage in the edit still getting its playback copy plays
        // slowly until it has one; say so.
        let rendering = video.preview.progress().map(|share| {
            format!(
                "Rendering a preview: {:.0}%. It plays perfectly once rendered.",
                share * 100.
            )
        });
        let copying = self.store.as_ref().ok().and_then(|store| {
            let making = store.copies.making?;
            let used = video
                .doc
                .tracks
                .iter()
                .flat_map(|t| t.clips.iter())
                .find(|c| c.source.asset() == making)?;
            let share = store.copies.progress(making).unwrap_or(0.);
            Some(format!(
                "Making a smooth-playback copy of {}: {:.0}%. Until it is ready, playing and jumping may be slow.",
                used.name,
                share * 100.
            ))
        });
        let adding = self.adding_overlay(cx);
        div()
            .size_full()
            .relative()
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
                    .when(!empty, |el| el.children(picture))
                    .relative()
                    .children(rendering.or(copying).map(|text| {
                        div()
                            .absolute()
                            .left(px(12.))
                            .bottom(px(12.))
                            .max_w(px(420.))
                            .px_3()
                            .py_1p5()
                            .rounded_md()
                            .bg(hsla(0., 0., 0., 0.7))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(text)
                    })),
            )
            .child(transport)
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(self.timeline(window, cx)),
            )
            .children(adding)
            .into_any_element()
    }
}
