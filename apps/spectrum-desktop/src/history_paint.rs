//! Drawing the History view's tree: a line from each revision to the next,
//! a dot for each revision (a preview once zoomed in far enough), the
//! person's revision and the selected one marked, and
//! labels for each branch and for where each session is. Only what is on
//! screen is drawn, so a history of thousands stays quick.
use crate::{
    history_view::{COLUMN, PREVIEWS_FROM, ROW},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::FluentBuilder, *};

/// Agents' revisions and places are drawn in this color; the person's in
/// the text color.
pub const AGENT: u32 = 0x8ab4f8;
/// The ring around the selected revision.
const PICKED: u32 = 0xf2c94c;

/// A revision's dot, in points.
pub fn node_radius(zoom: f32) -> f32 {
    (4.5 * zoom.sqrt()).clamp(2., 9.)
}

/// A revision's preview card when zoomed in, in points.
pub fn card_size(zoom: f32) -> (f32, f32) {
    (COLUMN * zoom * 0.84, ROW * zoom * 0.84)
}

/// Labels show once rows are at least this tall.
const LABELS_FROM: f32 = 14.;

impl Workspace {
    /// The History mode's main area.
    pub fn history_main(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let bounds_slot = self.history.bounds.clone();
        let base = div()
            .id("history-view")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(BACKGROUND))
            .child(
                canvas(
                    move |bounds, _, _| *bounds_slot.borrow_mut() = bounds,
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        let Some(shown) = &self.history.shown else {
            if self.history_asset().is_some() {
                self.load_history(window, cx);
            }
            return base
                .child(
                    div()
                        .absolute()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(rgb(FAINT))
                        .child("Reading history…"),
                )
                .into_any_element();
        };
        let view = &self.history;
        let layout = shown.layout.clone();
        let (pan, zoom) = (view.pan, view.zoom);
        let current = shown.history.current;
        let selected = view.selected;
        let cards = COLUMN * zoom >= PREVIEWS_FROM;
        let area = *self.history.bounds.borrow();
        let (width, height) = (f32::from(area.size.width), f32::from(area.size.height));
        let seen = |x: f32, y: f32, margin: f32| {
            (-margin..width + margin).contains(&x) && (-margin..height + margin).contains(&y)
        };
        let paint_layout = layout.clone();
        let tree = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let origin = bounds.origin;
                let at = |depth: u32, lane: u32| {
                    point(
                        origin.x + px(pan.0 + depth as f32 * COLUMN * zoom),
                        origin.y + px(pan.1 + lane as f32 * ROW * zoom),
                    )
                };
                let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let nodes = &paint_layout.nodes;
                // Lines first, under the dots, all in one path.
                let mut lines = PathBuilder::stroke(px((1.5 * zoom.sqrt()).clamp(1., 2.5)));
                let mut any = false;
                for node in nodes {
                    let Some(parent) = node.parent.map(|p| &nodes[p]) else {
                        continue;
                    };
                    let from = at(parent.depth, parent.lane);
                    let to = at(node.depth, node.lane);
                    let left = f32::from(from.x - origin.x);
                    let right = f32::from(to.x - origin.x);
                    let top = f32::from(from.y - origin.y);
                    let bottom = f32::from(to.y - origin.y);
                    if right < -COLUMN
                        || left > width + COLUMN
                        || bottom < -ROW
                        || top > height + ROW
                    {
                        continue;
                    }
                    lines.move_to(from);
                    if from.y != to.y {
                        lines.line_to(point(from.x, to.y));
                    }
                    lines.line_to(to);
                    any = true;
                }
                if any && let Ok(path) = lines.build() {
                    window.paint_path(path, rgb(0x3a3a3a));
                }
                if cards {
                    return;
                }
                let radius = node_radius(zoom);
                for node in nodes {
                    let center = at(node.depth, node.lane);
                    let (x, y) = (
                        f32::from(center.x - origin.x),
                        f32::from(center.y - origin.y),
                    );
                    if !(-radius..width + radius).contains(&x)
                        || !(-radius..height + radius).contains(&y)
                    {
                        continue;
                    }
                    let color = if node.agent {
                        rgb(AGENT)
                    } else if node.name.is_some() {
                        rgb(TEXT)
                    } else {
                        rgb(MUTED)
                    };
                    let dot = Bounds::centered_at(center, size(px(radius * 2.), px(radius * 2.)));
                    window.paint_quad(fill(dot, color).corner_radii(px(radius)));
                    let ring = |grow: f32, color: Rgba, window: &mut Window| {
                        let r = radius + grow;
                        let bounds = Bounds::centered_at(center, size(px(r * 2.), px(r * 2.)));
                        window.paint_quad(
                            outline(bounds, color, BorderStyle::Solid).corner_radii(px(r)),
                        );
                    };
                    if node.id == current {
                        ring(3., rgb(0xffffff), window);
                    }
                    if Some(node.id) == selected {
                        ring(6., rgb(PICKED), window);
                    }
                }
            },
        )
        .absolute()
        .size_full();
        // Previews, branch names, and places, as elements over the tree.
        let mut overlays: Vec<AnyElement> = Vec::new();
        let place = |depth: u32, lane: u32| view.place_of(depth, lane);
        if cards {
            let (card_w, card_h) = card_size(zoom);
            for node in &layout.nodes {
                let (x, y) = place(node.depth, node.lane);
                if !seen(x, y, card_w) {
                    continue;
                }
                let border = if Some(node.id) == selected {
                    rgb(PICKED)
                } else if node.id == current {
                    rgb(0xffffff)
                } else if node.agent {
                    rgb(AGENT)
                } else {
                    rgb(BORDER)
                };
                let preview = view.previews.get(&node.id).cloned();
                overlays.push(
                    div()
                        .absolute()
                        .left(px(x - card_w / 2.))
                        .top(px(y - card_h / 2.))
                        .w(px(card_w))
                        .h(px(card_h))
                        .rounded(px(4.))
                        .border_2()
                        .border_color(border)
                        .bg(rgb(SURFACE))
                        .overflow_hidden()
                        .when_some(preview, |el, image| {
                            el.child(img(image).size_full().object_fit(ObjectFit::Contain))
                        })
                        .into_any_element(),
                );
            }
        }
        if ROW * zoom >= LABELS_FROM {
            let offset = if cards {
                card_size(zoom).0 / 2. + 8.
            } else {
                node_radius(zoom) + 8.
            };
            for branch in &layout.branches {
                let tip = &layout.nodes[branch.tip];
                let (x, y) = place(tip.depth, tip.lane);
                if !seen(x, y, 240.) {
                    continue;
                }
                overlays.push(
                    div()
                        .absolute()
                        .left(px(x + offset))
                        .top(px(y - 9.))
                        .h(px(18.))
                        .flex()
                        .items_center()
                        .text_xs()
                        .whitespace_nowrap()
                        .text_color(rgb(if branch.name.is_some() { TEXT } else { MUTED }))
                        .child(layout.branch_title(branch))
                        .into_any_element(),
                );
            }
            // Where each session is: the person, and each agent.
            for session in &shown.history.sessions {
                if session.actor.id == "spectrum:reader" {
                    continue;
                }
                let Some(node) = layout.node(session.cursor) else {
                    continue;
                };
                let (x, y) = place(node.depth, node.lane);
                if !seen(x, y, 120.) {
                    continue;
                }
                let agent = !matches!(session.actor.kind, spectrum_document::ActorKind::Human);
                let lift = if cards {
                    card_size(zoom).1 / 2. + 22.
                } else {
                    node_radius(zoom) + 24.
                };
                overlays.push(
                    div()
                        .absolute()
                        .left(px(x - 40.))
                        .top(px(y - lift))
                        .w(px(80.))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_2()
                                .h(px(18.))
                                .flex()
                                .items_center()
                                .rounded_full()
                                .text_xs()
                                .bg(rgb(if agent { AGENT } else { TEXT }))
                                .text_color(rgb(BACKGROUND))
                                .child(session.actor.display_name.clone()),
                        )
                        .into_any_element(),
                );
            }
        }
        base.child(tree)
            .children(overlays)
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                this.history_scroll(event, window, cx)
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, _| {
                    this.history_press(event.position)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.history_drag(event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.history_release(event, window, cx)
                }),
            )
            .into_any_element()
    }
}
