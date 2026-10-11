//! The image editor's zoom: to 100% and a two-finger pan on the edited
//! photo, waiting each time for the part on screen to show sharp. And
//! switching between Crop and Color: the screen never shows a frame drawn
//! for the other view, so nothing stretches or jumps.
use super::{
    Fixtures, Outcome,
    scenarios::{This, ms, outcome, pause, until},
};
use crate::workspace::{Mode, Open, Workspace};
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

/// Whether the frame on screen was drawn for what the view shows now: in
/// Crop, the whole frame (or the cropped one placed where it sits); in
/// other modes, the edits as they are.
fn shows_right_frame(ws: &Workspace) -> bool {
    let wanted = ws.view_adjustments();
    let Some(shown) = ws.image.preview.as_ref().and_then(|p| p.displayed.clone()) else {
        return false;
    };
    if ws.mode == Mode::Crop {
        let whole = spectrum_image::Adjustments {
            crop: None,
            ..shown.clone()
        };
        whole == wanted
    } else {
        shown == wanted
    }
}

/// Where the picture sits on screen: the whole frame in Crop, the image in
/// the other modes.
fn placement(ws: &Workspace, window: &Window) -> Option<Bounds<Pixels>> {
    if ws.mode == Mode::Crop {
        Some(ws.frame_rect())
    } else {
        ws.image_rect(window).map(|(rect, _)| rect)
    }
}

fn moved(a: Option<Bounds<Pixels>>, b: Option<Bounds<Pixels>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let far = |x: Pixels, y: Pixels| f32::from(x - y).abs() > 1.;
            far(a.origin.x, b.origin.x)
                || far(a.origin.y, b.origin.y)
                || far(a.size.width, b.size.width)
                || far(a.size.height, b.size.height)
        }
        _ => true,
    }
}

pub(super) async fn crop_switch(
    this: &This,
    cx: &mut AsyncWindowContext,
    fixtures: &Fixtures,
) -> Outcome {
    let (project, photo) = (fixtures.project, fixtures.edited_photo);
    this.update_in(cx, |ws, window, cx| {
        ws.enter_project(project, window, cx);
        ws.open_item(Open::Image(photo), window, cx);
    })
    .ok();
    until(this, cx, 30_000, Workspace::image_idle).await;
    crate::perf::drain();
    let mut settles = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    for round in 0..3 {
        for (mode, recrop) in [(Mode::Crop, false), (Mode::Color, true)] {
            let started = Instant::now();
            let first = this
                .update_in(cx, |ws, window, cx| {
                    if recrop {
                        // A new crop made in Crop, as a drag of its corner does.
                        let mut crop = ws.image.adjust.crop.unwrap_or_default();
                        crop.width = (crop.width - 0.02 * (round + 1) as f32).max(0.2);
                        ws.image.adjust.crop = Some(crop);
                        ws.schedule_color_edit(window, cx);
                    }
                    ws.set_mode(mode, window, cx);
                    (shows_right_frame(ws), placement(ws, window))
                })
                .ok();
            let (right, placed) = first.unwrap_or((false, None));
            if !right {
                problems.push(format!(
                    "switching to {} showed a frame drawn for the other view",
                    mode.label()
                ));
            }
            match until(this, cx, 20_000, Workspace::image_idle).await {
                Some(_) => settles.push(("settle", ms(started.elapsed()))),
                None => problems.push(format!("{} never showed sharp", mode.label())),
            }
            // Drawn where it ends up from the first frame: nothing shifts.
            pause(cx, 50).await;
            let settled = this
                .update_in(cx, |ws, window, _| placement(ws, window))
                .ok()
                .flatten();
            if moved(placed, settled) {
                problems.push(format!(
                    "switching to {} placed the picture at {placed:?}, then moved it to {settled:?}",
                    mode.label()
                ));
            }
        }
    }
    problems.dedup();
    outcome(
        "crop_switch",
        settles,
        (!problems.is_empty()).then(|| problems.join("; ")),
    )
}
