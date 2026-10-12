//! The History mode's sidebar: Back and Forward, finding a branch (the
//! palette, ⌘P), the revision the person is on (a sharp preview of all of
//! it, what it did, who and when), the name of their branch, and the agents
//! they can watch.
use crate::{
    controls::group,
    history_paint::AGENT,
    theme::*,
    workspace::{SIDEBAR_WIDTH, Workspace},
};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
};
use spectrum_document::ActorKind;

/// The sidebar's content width, in points.
const CONTENT: f32 = SIDEBAR_WIDTH - 30.;
/// The tallest the large preview grows; a tall picture narrows instead.
const PREVIEW_HEIGHT: f32 = 300.;

/// "just now", "5 min ago", "3 h ago", or days.
pub fn when(at_ms: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let seconds = (now - at_ms).max(0) / 1000;
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

/// A picture's size fitted to the sidebar's width, or to the most height,
/// keeping its shape.
fn fitted(width: f32, height: f32) -> (f32, f32) {
    let ratio = height / width.max(1.);
    let tall = CONTENT * ratio;
    if tall <= PREVIEW_HEIGHT {
        (CONTENT, tall)
    } else {
        (PREVIEW_HEIGHT / ratio, PREVIEW_HEIGHT)
    }
}

fn dot(color: u32) -> Div {
    div()
        .size(px(8.))
        .flex_shrink_0()
        .rounded_full()
        .bg(rgb(color))
}

impl Workspace {
    pub fn history_sidebar(&mut self, cx: &mut Context<Self>) -> Div {
        let mut panel = div().flex().flex_col().gap_6();
        if let Some(watching) = &self.history.watching {
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .bg(rgb(SURFACE))
                    .text_sm()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(dot(AGENT))
                            .child(format!("Watching {}", watching.name)),
                    )
                    .child(
                        Button::new("stop-watching")
                            .ghost()
                            .xsmall()
                            .label("Stop")
                            .on_click(cx.listener(|this, _, _, cx| this.stop_watching(cx))),
                    ),
            );
        }
        // The branch the person is on, as the canvas shows its tool: a
        // button that opens the branch finder.
        let branch = self.history.shown.as_ref().and_then(|shown| {
            let lane = shown.layout.node(shown.history.current)?.lane;
            let branch = shown
                .layout
                .branches
                .iter()
                .find(|b| shown.layout.nodes[b.tip].lane == lane)?;
            Some(shown.layout.branch_title(branch))
        });
        panel = panel.child(
            div()
                .id("find-branch")
                .h(px(34.))
                .px_3()
                .flex()
                .items_center()
                .gap_2p5()
                .rounded_md()
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(SURFACE))
                .hover(|el| el.bg(rgb(HOVER)))
                .cursor_pointer()
                .child(
                    crate::icons::icon("history")
                        .size(px(14.))
                        .text_color(rgb(MUTED)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .child(branch.unwrap_or_else(|| "Branches".into())),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .child(crate::tools::SHORTCUT),
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_branches(window, cx))),
        );
        panel = panel.child(
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("history-back")
                        .small()
                        .flex_1()
                        .label("Back")
                        .disabled(self.history.back.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.history_back(window, cx))),
                )
                .child(
                    Button::new("history-forward")
                        .small()
                        .flex_1()
                        .label("Forward")
                        .disabled(self.history.forward.is_empty())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.history_forward(window, cx)),
                        ),
                ),
        );
        let Some(shown) = &self.history.shown else {
            return panel;
        };
        let current = shown.history.current;
        if let Some(node) = shown.layout.node(current) {
            // The last preview stays until this one's is ready.
            let preview = self.history.large.as_ref().map(|(_, image)| image.clone());
            let (width, height) = preview.as_ref().map_or((CONTENT, CONTENT * 0.66), |image| {
                let size = image.size(0);
                fitted(size.width.0 as f32, size.height.0 as f32)
            });
            panel = panel.child(
                group("You are here", None)
                    .child(
                        div().w_full().flex().justify_center().child(
                            div()
                                .w(px(width))
                                .h(px(height))
                                .rounded(px(4.))
                                .bg(rgb(SURFACE))
                                .overflow_hidden()
                                .when_some(preview, |el, image| {
                                    el.child(img(image).size_full().object_fit(ObjectFit::Fill))
                                }),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(node.label.clone().unwrap_or_else(|| "Edit".into())),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(dot(if node.agent { AGENT } else { TEXT }))
                            .child(format!("{} · {}", node.actor, when(node.at_ms))),
                    ),
            );
        }
        panel = panel.child(
            group("Branch", None)
                .child({
                    // DEBUG (debug/mac-input only): field variants to compare on macOS.
                    let input = Input::new(&self.history.name);
                    match std::env::var("SPECTRUM_INPUT_VARIANT").as_deref() {
                        Ok("prefix") => input
                            .prefix(gpui_component::Icon::new(gpui_component::IconName::Search).small())
                            .into_any_element(),
                        Ok("cleanable") => input.cleanable(true).into_any_element(),
                        Ok("small") => input.small().into_any_element(),
                        _ => input.into_any_element(),
                    }
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .child("Names the branch you are on, from where it began."),
                ),
        );
        // Agents working on this asset, to watch as they work.
        let watched = self.history.watching.as_ref().map(|w| w.session);
        let agents: Vec<AnyElement> = shown
            .history
            .sessions
            .iter()
            .filter(|s| {
                !matches!(s.actor.kind, ActorKind::Human) && s.actor.id != "spectrum:reader"
            })
            .map(|s| {
                let (session, name) = (s.id, s.actor.display_name.clone());
                let watching = watched == Some(session);
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(dot(AGENT))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().text_sm().truncate().child(name.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(when(s.updated_at_ms)),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("watch-{session}")))
                            .xsmall()
                            .ghost()
                            .label(if watching { "Stop" } else { "Watch" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if watching {
                                    this.stop_watching(cx);
                                } else {
                                    this.watch_session(session, name.clone(), window, cx);
                                }
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        if !agents.is_empty() {
            panel = panel.child(
                group("Agents", None).child(div().flex().flex_col().gap_2().children(agents)),
            );
        }
        panel
    }
}
