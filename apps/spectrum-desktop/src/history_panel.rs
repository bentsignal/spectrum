//! The History mode's sidebar: search, the selected revision (its preview,
//! what it did, who and when, its name, going there, watching who is there),
//! and every branch, newest first.
use crate::{controls::group, history_paint::AGENT, theme::*, workspace::Workspace};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    input::Input,
};
use spectrum_document::ActorKind;

/// "just now", "5 min ago", "3 h ago", or the day.
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

impl Workspace {
    pub fn history_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
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
                            .child(div().size(px(8.)).rounded_full().bg(rgb(AGENT)))
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
        if let Some(node) = self.history.selected.and_then(|id| shown.layout.node(id)) {
            let id = node.id;
            let current = shown.history.current == id;
            let preview = self.history.previews.get(&id).cloned();
            // Sessions there other than the person: they can be watched.
            let others: Vec<(spectrum_document::SessionId, String)> = shown
                .history
                .sessions
                .iter()
                .filter(|s| {
                    s.cursor == id
                        && !matches!(s.actor.kind, ActorKind::Human)
                        && s.actor.id != "spectrum:reader"
                })
                .map(|s| (s.id, s.actor.display_name.clone()))
                .collect();
            let watched = self.history.watching.as_ref().map(|w| w.session);
            panel = panel.child(
                group(if current { "You are here" } else { "Revision" }, None)
                    .child(
                        div()
                            .w_full()
                            .h(px(150.))
                            .rounded(px(6.))
                            .bg(rgb(SURFACE))
                            .overflow_hidden()
                            .when_some(preview, |el, image| {
                                el.child(img(image).size_full().object_fit(ObjectFit::Contain))
                            }),
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
                            .child(div().size(px(8.)).rounded_full().bg(rgb(if node.agent {
                                AGENT
                            } else {
                                TEXT
                            })))
                            .child(format!("{} · {}", node.actor, when(node.at_ms))),
                    )
                    .child(Input::new(&self.history.name).small())
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(FAINT))
                            .child("Name it to mark a checkpoint and name its branch."),
                    )
                    .when(!current, |el| {
                        el.child(
                            Button::new("go-here")
                                .small()
                                .label("Go to this revision")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.go_to_revision(id, window, cx)
                                })),
                        )
                    })
                    .children(others.into_iter().filter(|(s, _)| Some(*s) != watched).map(
                        |(session, name)| {
                            let label = format!("Watch {name}");
                            Button::new(SharedString::from(format!("watch-{session}")))
                                .small()
                                .ghost()
                                .label(label)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.watch_session(session, name.clone(), window, cx)
                                }))
                        },
                    )),
            );
        }
        // Branches, newest first.
        let mut branches: Vec<_> = shown.layout.branches.iter().collect();
        branches.sort_by_key(|b| std::cmp::Reverse(shown.layout.nodes[b.tip].at_ms));
        // Each row ends in one branch; the person is on the branch of their row.
        let current_lane = shown.layout.node(shown.history.current).map(|n| n.lane);
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
                    .when(self.history.selected == Some(id), |el| el.bg(rgb(SELECTED)))
                    .child(
                        div()
                            .size(px(8.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(rgb(if tip.agent { AGENT } else { TEXT })),
                    )
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
                        this.select_revision(id, window, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        let _ = window;
        panel.child(group("Branches", None).child(div().flex().flex_col().gap_1().children(rows)))
    }
}
