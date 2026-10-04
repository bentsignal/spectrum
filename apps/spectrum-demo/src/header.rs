//! The title row above the main area: what is open, and actions on it.
use crate::{
    theme::*,
    workspace::{HEADER_HEIGHT, Mode, Open, Place, TRAFFIC_LIGHTS, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
};

impl Workspace {
    pub fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = |n: usize, noun: &str| -> SharedString {
            format!("{n} {noun}{}", if n == 1 { "" } else { "s" }).into()
        };
        let entries = self.store.as_ref().map_or(0, |s| s.entries.len());
        let (title, detail, grid): (SharedString, SharedString, bool) =
            match (self.place, self.open) {
                (Place::Home, _) if self.mode == Mode::Projects => {
                    let n = self.store.as_ref().map_or(0, |s| s.projects.len());
                    ("Projects".into(), count(n, "project"), false)
                }
                (Place::Home, _) => (self.view_name(), count(entries, "asset"), true),
                (_, Open::Overview) => (self.view_name(), count(entries, "asset"), true),
                (_, Open::Image(id)) => (self.asset_name(id), "Image".into(), false),
                (_, Open::Canvas(_)) => {
                    let (name, detail) = self.canvas.as_ref().map_or_else(
                        || (SharedString::default(), SharedString::default()),
                        |c| {
                            (
                                c.doc.name.clone().into(),
                                format!("Canvas · {} × {}", c.doc.width, c.doc.height).into(),
                            )
                        },
                    );
                    (name, detail, false)
                }
            };
        // Only the title area moves the window, so controls here drag normally.
        div()
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1p5()
            .pr_6()
            .child(
                self.drag_area("main-header", cx)
                    .flex_1()
                    .min_w_0()
                    .pl(px(if self.sidebar_right {
                        TRAFFIC_LIGHTS.max(24.)
                    } else {
                        24.
                    }))
                    .gap_3()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                    .child(div().text_sm().text_color(rgb(FAINT)).child(detail)),
            )
            .when(grid, |el| el.child(self.grid_controls(cx)))
            .when(matches!(self.open, Open::Image(_)), |el| {
                el.child(
                    Button::new("compare")
                        .ghost()
                        .small()
                        .label("Compare")
                        .selected(self.compare)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.compare = !this.compare;
                            cx.notify();
                        })),
                )
            })
            .when(
                matches!(self.open, Open::Canvas(_)) && self.canvas.is_some(),
                |el| {
                    let snapping = self.canvas.as_ref().is_some_and(|c| c.doc.snapping_enabled);
                    el.child(
                        Button::new("snapping")
                            .ghost()
                            .small()
                            .label("Snap")
                            .selected(snapping)
                            .tooltip("Snapping (⌘⇧;)")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_snapping(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("guides")
                            .ghost()
                            .small()
                            .label("Guides")
                            .selected(self.guides_visible)
                            .tooltip("Show guides (⌘;)")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_guides(cx))),
                    )
                },
            )
            .children(match self.open {
                Open::Image(id) | Open::Canvas(id) if self.place != Place::Home => Some(
                    Button::new("export")
                        .ghost()
                        .small()
                        .label("Export…")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.export_asset(id, window, cx)
                        })),
                ),
                _ => None,
            })
    }
}
