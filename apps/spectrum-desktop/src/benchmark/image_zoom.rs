//! The image editor's zoom: to 100% and a two-finger pan on the edited
//! photo, waiting each time for the part on screen to show sharp.
use super::{
    Fixtures, Outcome,
    scenarios::{This, ms, outcome, pause, until},
};
use crate::workspace::{Open, Workspace};
use gpui::*;
use std::time::{Duration, Instant};

/// Waits until the open image shows sharp everywhere on screen.
async fn sharp(this: &This, cx: &mut AsyncWindowContext, limit_ms: u64) -> Option<Duration> {
    let started = Instant::now();
    loop {
        let ready = this
            .update_in(cx, |ws, window, _| {
                ws.image_idle() && ws.image_detail_ready(window)
            })
            .unwrap_or(false);
        if ready {
            return Some(started.elapsed());
        }
        if started.elapsed() > Duration::from_millis(limit_ms) {
            return None;
        }
        pause(cx, 5).await;
    }
}

/// A two-finger pan sideways across the image.
async fn swipe(this: &This, cx: &mut AsyncWindowContext) {
    for step in 0..12 {
        this.update_in(cx, |ws, window, cx| {
            let center = ws.image.image_bounds.borrow().center();
            let event = ScrollWheelEvent {
                position: center,
                delta: ScrollDelta::Pixels(point(
                    px(-30.),
                    px(if step % 3 == 0 { 6. } else { 0. }),
                )),
                modifiers: Modifiers::default(),
                touch_phase: if step == 0 {
                    TouchPhase::Started
                } else {
                    TouchPhase::Moved
                },
            };
            ws.image_scroll(&event, window, cx);
        })
        .ok();
        pause(cx, 16).await;
    }
}

pub(super) async fn image_zoom(
    this: &This,
    cx: &mut AsyncWindowContext,
    fixtures: &Fixtures,
) -> Outcome {
    let (project, photo) = (fixtures.project, fixtures.photo);
    this.update_in(cx, |ws, window, cx| {
        ws.enter_project(project, window, cx);
        ws.open_item(Open::Image(photo), window, cx);
    })
    .ok();
    until(this, cx, 20_000, Workspace::image_idle).await;
    crate::perf::drain();
    let mut settles = Vec::new();
    let mut problem = None;
    for _ in 0..3 {
        this.update_in(cx, |ws, window, cx| ws.zoom_any(Some(1.), window, cx))
            .ok();
        match sharp(this, cx, 20_000).await {
            Some(took) => settles.push(("settle", ms(took))),
            None => problem = Some("the image never showed sharp at 100%".into()),
        }
        let before = this.read_with(cx, |ws, _| ws.image.view.pan).ok();
        swipe(this, cx).await;
        let after = this.read_with(cx, |ws, _| ws.image.view.pan).ok();
        if before == after {
            problem = Some("a two-finger swipe did not pan the zoomed image".into());
        }
        match sharp(this, cx, 20_000).await {
            Some(took) => settles.push(("settle", ms(took))),
            None => problem = Some("the image never showed sharp after panning".into()),
        }
        this.update_in(cx, |ws, window, cx| ws.zoom_any(None, window, cx))
            .ok();
        sharp(this, cx, 20_000).await;
    }
    outcome("image_zoom", settles, problem)
}
