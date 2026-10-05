//! The sidebar: a back button inside projects, the mode switcher, and the
//! current mode's content, with the tool first on every canvas mode.
use crate::{
    theme::*,
    workspace::{Mode, Open, Place, SIDEBAR_WIDTH, TRAFFIC_LIGHTS, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
};

impl Workspace {
    /// The fixed strip: a back button inside projects, then the mode tabs.
    fn strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let modes = self.modes();
        let back: Option<SharedString> = match (self.place, self.open) {
            (Place::Home, _) => None,
            (Place::Project(_), Open::Overview) => Some("Home".into()),
            (Place::Project(id), _) => self
                .store
                .as_ref()
                .ok()
                .and_then(|s| s.project_name(id))
                .map(|name| name.to_string().into()),
        };
        div()
            .px_3()
            .pb_3()
            .flex()
            .flex_col()
            .gap_2()
            .when(back.is_some() || !modes.is_empty(), |el| {
                el.border_b_1().border_color(rgb(BORDER))
            })
            .children(back.map(|label| {
                div()
                    .id("back")
                    // Matches the mode tabs' height so the divider stays put.
                    .h(px(36.))
                    .px_1()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .rounded_md()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .hover(|el| el.bg(rgb(HOVER)).text_color(rgb(TEXT)))
                    .child(Icon::new(IconName::ChevronLeft).small())
                    .child(div().truncate().child(label))
                    .on_click(cx.listener(|this, _, window, cx| this.back(window, cx)))
            }))
            .when(!modes.is_empty(), |el| el.child(self.mode_switcher(cx)))
    }

    pub fn sidebar(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match (self.place, self.mode) {
            (Place::Home, Mode::Projects) => self.projects_sidebar(cx).into_any_element(),
            (Place::Home, _) => self.library_sidebar(cx).into_any_element(),
            (_, _) if self.open == Open::Overview => self.project_sidebar(cx).into_any_element(),

            (_, Mode::Color) => self.color_sidebar(cx).into_any_element(),
            (_, Mode::Crop) => self.crop_sidebar(cx).into_any_element(),
            (_, Mode::Style) => self.style_sidebar(cx).into_any_element(),
            (_, Mode::Overview) => self.tool_sidebar(cx).into_any_element(),
            (_, Mode::Canvas) => self.canvas_settings(cx).into_any_element(),
            (_, Mode::Info) => self.info_sidebar(cx).into_any_element(),
            _ => self.layers_sidebar(cx).into_any_element(),
        };
        // Every canvas mode starts with the tool, so it can change from any
        // of them; Overview shows it with its settings.
        let content = if matches!(self.open, Open::Canvas(_))
            && self.place != Place::Home
            && self.mode != Mode::Overview
        {
            div()
                .flex()
                .flex_col()
                .gap_5()
                .child(self.tool_button(cx))
                .child(content)
                .into_any_element()
        } else {
            content
        };
        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(SIDEBAR))
            .map(|el| {
                if self.sidebar_right {
                    el.border_l_1()
                } else {
                    el.border_r_1()
                }
            })
            .border_color(rgb(BORDER))
            .child(
                self.drag_area("sidebar-header", cx)
                    .pl(px(if self.sidebar_right {
                        16.
                    } else {
                        TRAFFIC_LIGHTS
                    }))
                    .pr_3()
                    .justify_end()
                    .child(
                        Button::new("sidebar-side")
                            .ghost()
                            .small()
                            .icon(if self.sidebar_right {
                                IconName::PanelLeft
                            } else {
                                IconName::PanelRight
                            })
                            .tooltip(if self.sidebar_right {
                                "Move sidebar left"
                            } else {
                                "Move sidebar right"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_right = !this.sidebar_right;
                                cx.notify();
                            })),
                    ),
            )
            .child(self.strip(cx))
            .child(
                div()
                    .id(SharedString::from(format!("sidebar-{}", self.mode.label())))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_4()
                    .pt_4()
                    .pb_6()
                    .child(content),
            )
    }
}
