//! Before and after: the image with no adjustments beside the edited one.
use crate::{theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use std::sync::Arc;

impl Workspace {
    /// One labeled half of the comparison.
    pub fn compare_half(image: Option<Arc<RenderImage>>, label: &'static str) -> Div {
        let image = match image {
            Some(image) => img(image)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => div().into_any_element(),
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(image),
            )
            .child(div().text_xs().text_color(rgb(FAINT)).child(label))
    }
}
