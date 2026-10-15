//! Which kinds of asset a grid shows: a quiet list with counts, like a
//! Finder sidebar. A click shows only that kind (a click again, or All,
//! shows everything); Command-click adds or removes one; a right-click
//! offers the same. Only kinds the view holds are listed.
use crate::{theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable,
    menu::{ContextMenuExt, PopupMenuItem},
};
use spectrum_library::AssetKind;

const KINDS: [(AssetKind, &str); 5] = [
    (AssetKind::Image, "Images"),
    (AssetKind::Canvas, "Canvases"),
    (AssetKind::Footage, "Footage"),
    (AssetKind::Audio, "Audio"),
    (AssetKind::Video, "Videos"),
];

fn icon(kind: AssetKind) -> Icon {
    match kind {
        AssetKind::Image => Icon::new(IconName::Frame),
        AssetKind::Canvas => Icon::new(IconName::LayoutDashboard),
        AssetKind::Footage => crate::icons::icon("film"),
        AssetKind::Audio => crate::icons::icon("volume"),
        _ => crate::icons::icon("play"),
    }
}

impl Workspace {
    /// Shows only `kind`, or everything when it already is the only one.
    fn only_kind(&mut self, kind: AssetKind, cx: &mut Context<Self>) {
        let alone = KINDS
            .iter()
            .all(|(k, _)| (*k == kind) != self.hidden_kinds.contains(k));
        self.hidden_kinds = if alone {
            Vec::new()
        } else {
            KINDS
                .iter()
                .map(|(k, _)| *k)
                .filter(|k| *k != kind)
                .collect()
        };
        cx.notify();
    }

    /// Shows or hides one kind, keeping the others as they are.
    fn toggle_kind(&mut self, kind: AssetKind, cx: &mut Context<Self>) {
        match self.hidden_kinds.iter().position(|k| *k == kind) {
            Some(at) => {
                self.hidden_kinds.remove(at);
            }
            None => self.hidden_kinds.push(kind),
        }
        cx.notify();
    }

    pub fn kind_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let counts: Vec<(AssetKind, &str, usize)> = {
            let entries = self
                .store
                .as_ref()
                .map(|s| s.entries.as_slice())
                .unwrap_or(&[]);
            KINDS
                .iter()
                .map(|(kind, label)| {
                    let count = entries.iter().filter(|e| e.asset.kind == *kind).count();
                    (*kind, *label, count)
                })
                .filter(|(_, _, count)| *count > 0)
                .collect()
        };
        let total: usize = counts.iter().map(|c| c.2).sum();
        let everything = self.hidden_kinds.is_empty();
        let row = |id: SharedString, icon: Icon, label: &str, count: usize, on: bool| {
            div()
                .id(id)
                .h(px(30.))
                .px_2p5()
                .flex()
                .items_center()
                .gap_2p5()
                .rounded_md()
                .text_sm()
                .text_color(rgb(if on { TEXT } else { FAINT }))
                .when(on, |el| el.bg(rgb(SELECTED)))
                .when(!on, |el| el.hover(|el| el.bg(rgb(HOVER))))
                .child(icon.small().text_color(rgb(if on { MUTED } else { FAINT })))
                .child(div().flex_1().child(label.to_string()))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .child(count.to_string()),
                )
        };
        let view = cx.entity();
        let rows = counts.into_iter().map(|(kind, label, count)| {
            let shown = !self.hidden_kinds.contains(&kind);
            let menu = {
                let view = view.clone();
                move |menu: gpui_component::menu::PopupMenu, _: &mut Window, _: &mut Context<_>| {
                    let (only, toggle) = (view.clone(), view.clone());
                    menu.item(PopupMenuItem::new(format!("Show only {label}")).on_click(
                        move |_, _, cx| only.update(cx, |this, cx| this.only_kind(kind, cx)),
                    ))
                    .item(
                        PopupMenuItem::new(if shown {
                            format!("Hide {label}")
                        } else {
                            format!("Also show {label}")
                        })
                        .on_click(move |_, _, cx| {
                            toggle.update(cx, |this, cx| this.toggle_kind(kind, cx))
                        }),
                    )
                }
            };
            row(
                format!("kind-{label}").into(),
                icon(kind),
                label,
                count,
                shown && !everything,
            )
            .when(everything, |el| el.text_color(rgb(TEXT)))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.modifiers().secondary() || event.modifiers().shift {
                    this.toggle_kind(kind, cx)
                } else {
                    this.only_kind(kind, cx)
                }
            }))
            .context_menu(menu)
        });
        div()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(
                row(
                    "kind-all".into(),
                    Icon::new(IconName::LayoutDashboard),
                    "All kinds",
                    total,
                    everything,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.hidden_kinds.clear();
                    cx.notify();
                })),
            )
            .children(rows)
    }
}
