//! The History mode's sidebar: Back and Forward, search, the revision the
//! person is on (a sharp preview of all of it, what it did, who and when),
//! the name of their branch, the agents they can watch, and every branch,
//! newest first. Clicking a branch goes to its newest revision.
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
        let matches = self.history.matches.len();
        let searching = !self.history.search.read(cx).value().trim().is_empty();
        panel = panel.child(
            group("Search", None)
                .child(Input::new(&self.history.search).small())
                .when(searching, |el| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(match matches {
                                0 => "No revisions match".to_string(),
                                1 => "1 revision".to_string(),
                                n => format!("{n} revisions"),
                            })
                            .when(matches > 0, |el| {
                                el.child(
                                    Button::new("next-match")
                                        .ghost()
                                        .xsmall()
                                        .label("Next")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.next_history_match(window, cx)
                                        })),
                                )
                            }),
                    )
                }),
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
                .child(Input::new(&self.history.name).small())
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
        // Branches, newest first; the person's is the one in their row.
        let mut branches: Vec<_> = shown.layout.branches.iter().collect();
        branches.sort_by_key(|b| std::cmp::Reverse(shown.layout.nodes[b.tip].at_ms));
        let current_lane = shown.layout.node(current).map(|n| n.lane);
        let rows: Vec<AnyElement> = branches
            .into_iter()
            .take(200)
            .map(|branch| {
                let tip = &shown.layout.nodes[branch.tip];
                let id = tip.id;
                let here = current_lane == Some(tip.lane);
                div()
                    .id(SharedString::from(format!("branch-{id}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|el| el.bg(rgb(HOVER)))
                    .when(here, |el| el.bg(rgb(SELECTED)))
                    .child(dot(if tip.agent { AGENT } else { TEXT }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .child(shown.layout.branch_title(branch)),
                            )
                            .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                                "{}{}",
                                when(tip.at_ms),
                                if here { " · you are on it" } else { "" }
                            ))),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.center_on_revision(id);
                        this.go_to_revision(id, window, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        panel.child(group("Branches", None).child(div().flex().flex_col().gap_1().children(rows)))
    }
}
