//! The sidebar's mode switcher: a dropdown naming the current mode. Its
//! list opens beside the sidebar, so the mode's content stays in view, and
//! shows each mode's Command+number shortcut; holding Command for a moment
//! opens it, so the shortcuts are there when you need them and out of the
//! way when you already know them.
use crate::{
    theme::*,
    workspace::{SIDEBAR_WIDTH, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{Icon, IconName, Sizable};
use std::time::Duration;

/// How long Command is held before the list opens.
const HOLD: Duration = Duration::from_millis(500);

/// "⌘1" on a Mac, "Ctrl+1" elsewhere.
fn shortcut(index: usize) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{}", index + 1)
    } else {
        format!("Ctrl+{}", index + 1)
    }
}

impl Workspace {
    pub fn mode_switcher(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let modes = self.modes();
        let current = self.mode;
        let open = self.mode_menu;
        let list = open.then(|| {
            let items = modes.iter().enumerate().map(|(index, mode)| {
                let mode = *mode;
                let on = mode == current;
                div()
                    .id(("mode-item", index))
                    .h(px(34.))
                    .px_2p5()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .rounded_md()
                    .text_sm()
                    .text_color(rgb(if on { TEXT } else { MUTED }))
                    .when(on, |el| el.bg(rgb(SELECTED)))
                    .when(!on, |el| {
                        el.hover(|el| el.bg(rgb(HOVER)).text_color(rgb(TEXT)))
                    })
                    .child(Icon::new(mode.icon()).small())
                    .child(div().flex_1().child(mode.label()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(FAINT))
                            .child(shortcut(index)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.mode_menu = false;
                        this.mode_menu_held = false;
                        this.set_mode(mode, window, cx);
                    }))
            });
            // Beside the sidebar, level with the switcher.
            let at = *self.mode_switcher_bounds.borrow();
            let (x, corner) = if self.sidebar_right {
                (at.left() - px(20.), Corner::TopRight)
            } else {
                (at.right() + px(20.), Corner::TopLeft)
            };
            deferred(
                anchored()
                    .position(point(x, at.top()))
                    .anchor(corner)
                    .snap_to_window()
                    .child(
                        div()
                            .id("mode-list")
                            .occlude()
                            .w(px(SIDEBAR_WIDTH - 24.))
                            .p_1()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(0x333333))
                            .bg(rgb(SURFACE))
                            .shadow_lg()
                            .children(items)
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.mode_menu = false;
                                this.mode_menu_held = false;
                                cx.notify();
                            })),
                    ),
            )
            .with_priority(1)
        });
        div()
            .child(
                div()
                    .id("mode-switcher")
                    .h(px(36.))
                    .px_2p5()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .rounded_lg()
                    .bg(rgb(SURFACE))
                    .text_sm()
                    .hover(|el| el.bg(rgb(HOVER)))
                    .child({
                        let slot = self.mode_switcher_bounds.clone();
                        canvas(
                            move |bounds, _, _| *slot.borrow_mut() = bounds,
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full()
                    })
                    .relative()
                    .child(Icon::new(current.icon()).small())
                    .child(div().flex_1().child(current.label()))
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(MUTED)),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.mode_menu = !this.mode_menu;
                        this.mode_menu_held = false;
                        cx.notify();
                    })),
            )
            .children(list)
    }

    /// Command pressed alone starts the hold; letting go closes a list the
    /// hold opened.
    pub fn modifiers_changed(
        &mut self,
        modifiers: &Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hold_token += 1;
        let alone = modifiers.secondary() && !modifiers.shift && !modifiers.alt;
        if !modifiers.secondary() && self.mode_menu_held {
            self.mode_menu = false;
            self.mode_menu_held = false;
            cx.notify();
        }
        if !alone || self.modes().is_empty() {
            return;
        }
        let token = self.hold_token;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(HOLD).await;
            this.update_in(cx, |this, window, cx| {
                if this.hold_token == token && window.modifiers().secondary() && !this.mode_menu {
                    this.mode_menu = true;
                    this.mode_menu_held = true;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}
