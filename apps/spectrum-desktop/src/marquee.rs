//! Drag-box selection in asset grids, scrolling when the pointer nears an edge.
use crate::workspace::Workspace;
use gpui::*;
use std::time::Duration;

/// Distance from the grid's top or bottom edge where dragging scrolls.
const EDGE: f32 = 48.;

/// A drag box. `start` is in scrolled content coordinates so it stays anchored
/// while the grid scrolls; `pointer` is the current window position.
#[derive(Clone, Copy)]
pub struct Marquee {
    pub start: Point<Pixels>,
    pub pointer: Point<Pixels>,
}

impl Workspace {
    fn content_to_window(&self, point: Point<Pixels>) -> Point<Pixels> {
        point + self.grid_bounds.borrow().origin + self.grid_scroll.offset()
    }

    pub fn start_marquee(
        &mut self,
        position: Point<Pixels>,
        additive: bool,
        cx: &mut Context<Self>,
    ) {
        if !additive {
            self.selection.clear();
        }
        let start = position - self.grid_bounds.borrow().origin - self.grid_scroll.offset();
        self.marquee = Some(Marquee {
            start,
            pointer: position,
        });
        cx.notify();
    }

    pub fn end_marquee(&mut self, cx: &mut Context<Self>) {
        self.marquee = None;
        cx.notify();
    }

    /// The drag box in window coordinates.
    pub fn marquee_area(&self) -> Option<Bounds<Pixels>> {
        let marquee = self.marquee?;
        let start = self.content_to_window(marquee.start);
        let end = marquee.pointer;
        Some(Bounds::from_corners(
            point(start.x.min(end.x), start.y.min(end.y)),
            point(start.x.max(end.x), start.y.max(end.y)),
        ))
    }

    pub fn move_marquee(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(marquee) = &mut self.marquee else {
            return;
        };
        marquee.pointer = position;
        self.select_in_marquee(cx);
        if !self.autoscrolling && self.scroll_speed() != 0. {
            self.autoscrolling = true;
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let more = this
                        .update(cx, |this, cx| this.autoscroll_step(cx))
                        .unwrap_or(false);
                    if !more {
                        break;
                    }
                }
            })
            .detach();
        }
    }

    fn select_in_marquee(&mut self, cx: &mut Context<Self>) {
        let Some(area) = self.marquee_area() else {
            return;
        };
        let bounds = self.card_bounds.borrow();
        self.selection = self
            .visible(cx)
            .into_iter()
            .map(|e| e.asset.id)
            .filter(|id| bounds.get(id).is_some_and(|b| b.intersects(&area)))
            .collect();
        drop(bounds);
        cx.notify();
    }

    /// Pixels per frame to scroll: faster the further past the edge zone.
    fn scroll_speed(&self) -> f32 {
        let Some(marquee) = self.marquee else {
            return 0.;
        };
        let bounds = *self.grid_bounds.borrow();
        let y = f32::from(marquee.pointer.y);
        let (top, bottom) = (f32::from(bounds.top()), f32::from(bounds.bottom()));
        if y > bottom - EDGE {
            (y - (bottom - EDGE)).min(EDGE * 2.) * 0.4 + 2.
        } else if y < top + EDGE {
            -((top + EDGE - y).min(EDGE * 2.) * 0.4 + 2.)
        } else {
            0.
        }
    }

    fn autoscroll_step(&mut self, cx: &mut Context<Self>) -> bool {
        let speed = self.scroll_speed();
        if speed == 0. {
            self.autoscrolling = false;
            return false;
        }
        let offset = self.grid_scroll.offset();
        let max = self.grid_scroll.max_offset().height;
        let y = (offset.y - px(speed)).clamp(-max, px(0.));
        self.grid_scroll.set_offset(point(offset.x, y));
        self.select_in_marquee(cx);
        true
    }
}
