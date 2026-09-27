//! A project's Assets mode: a compact list for opening items in the main area.
use crate::{
    store::Thumb,
    theme::*,
    workspace::{Open, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
};

impl Workspace {
    pub fn project_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let row = |id: SharedString, selected: bool| {
            div()
                .id(id)
                .h(px(40.))
                .px_1p5()
                .flex()
                .items_center()
                .gap_2p5()
                .rounded_md()
                .text_sm()
                .when(selected, |el| el.bg(rgb(SELECTED)))
                .when(!selected, |el| el.hover(|el| el.bg(rgb(HOVER))))
        };
        let icon_tile = |icon: IconName| {
            div()
                .size(px(30.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .bg(rgb(SURFACE))
                .child(Icon::new(icon).small().text_color(rgb(MUTED)))
        };
        let items: Vec<AnyElement> = self
            .visible(cx)
            .into_iter()
            .map(|entry| {
                let id = entry.asset.id;
                let thumb = self.store.as_ref().ok().and_then(|s| s.thumbs.get(&id));
                let tile = div()
                    .size(px(30.))
                    .flex_shrink_0()
                    .rounded_sm()
                    .overflow_hidden()
                    .bg(rgb(SURFACE))
                    .children(match thumb {
                        Some(Thumb::Ready(path)) => {
                            Some(img(path.clone()).size_full().object_fit(ObjectFit::Cover))
                        }
                        _ => None,
                    });
                let kind = if entry.asset.kind == "canvas" {
                    "Canvas"
                } else {
                    "Image"
                };
                row(format!("item-{id}").into(), self.open == Open::Image(id))
                    .child(tile)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().child(entry.asset.name.clone()))
                            .child(div().text_xs().text_color(rgb(FAINT)).child(kind)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_asset_from_list(id, window, cx)
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                Button::new("import")
                    .primary()
                    .icon(IconName::Plus)
                    .label(if self.importing > 0 {
                        format!("Importing {}…", self.importing)
                    } else {
                        "Import assets".into()
                    })
                    .loading(self.importing > 0)
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| this.choose_import(window, cx))),
            )
            .child(
                Input::new(&self.search)
                    .prefix(Icon::new(IconName::Search).small().text_color(rgb(MUTED)))
                    .cleanable(true),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        row("overview".into(), self.open == Open::Overview)
                            .child(icon_tile(IconName::LayoutDashboard))
                            .child("Overview")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_item(Open::Overview, window, cx)
                            })),
                    )
                    .children(items)
                    .child(
                        row("sample".into(), self.open == Open::Sample)
                            .child(icon_tile(IconName::Frame))
                            .child(div().flex_1().child("Spring poster"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(FAINT))
                                    .pr_1()
                                    .child("Sample"),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_item(Open::Sample, window, cx)
                            })),
                    ),
            )
    }

    fn open_asset_from_list(
        &mut self,
        id: spectrum_library::AssetId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let canvas = self
            .store
            .as_ref()
            .ok()
            .and_then(|s| s.entries.iter().find(|e| e.asset.id == id))
            .is_some_and(|e| e.asset.kind == "canvas");
        if canvas {
            self.notify_error(
                "Canvases open here once the canvas editor moves to this app.",
                window,
                cx,
            );
        } else {
            self.open_item(Open::Image(id), window, cx);
        }
    }
}
