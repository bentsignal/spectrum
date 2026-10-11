//! The benchmark's scenarios: what a person does, as synthetic input.
use super::{Fixtures, Outcome};
use crate::workspace::{Open, Workspace};
use gpui::*;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub(super) type This = WeakEntity<Workspace>;

pub(super) async fn run_all(
    this: &This,
    cx: &mut AsyncWindowContext,
    fixtures: &Fixtures,
) -> Vec<Outcome> {
    // Some X servers draw a new window only once it is resized, as a window
    // manager would; a fixed size also keeps every run alike.
    for width in [1281., 1280.] {
        cx.update(|window, _| {
            window.resize(size(px(width), px(820.)));
            window.activate_window();
            window.refresh();
        })
        .ok();
        pause(cx, 300).await;
    }
    // Let the library load.
    pause(cx, 1200).await;
    let only = super::only();
    // Each scenario names itself on stderr as it starts, so a crash shows
    // where it happened.
    let wanted = |name: &str| {
        let wanted = only
            .as_ref()
            .is_none_or(|only| only.iter().any(|o| o == name));
        if wanted {
            eprintln!("benchmark: {name}");
        }
        wanted
    };
    let mut outcomes = Vec::new();
    if wanted("image_open") {
        outcomes.push(image_open("image_open", this, cx, fixtures, fixtures.photo).await);
    }
    if wanted("image_scrub") {
        outcomes.push(image_scrub("image_scrub", this, cx).await);
    }
    if wanted("edited_open") {
        outcomes.push(image_open("edited_open", this, cx, fixtures, fixtures.edited_photo).await);
    }
    if wanted("edited_scrub") {
        outcomes.push(image_scrub("edited_scrub", this, cx).await);
    }
    if wanted("canvas_open") {
        outcomes.push(canvas_open("canvas_open", this, cx, fixtures, fixtures.canvas).await);
    }
    if wanted("canvas_drag") {
        outcomes.push(canvas_drag("canvas_drag", this, cx, fixtures.photo_layer).await);
    }
    if wanted("canvas_zoom") {
        outcomes.push(canvas_zoom("canvas_zoom", this, cx).await);
    }
    if wanted("text_size") {
        outcomes.push(text_size(this, cx, fixtures).await);
    }
    if wanted("brush_undo") {
        outcomes.push(brush_undo("brush_undo", this, cx).await);
    }
    if wanted("heavy_open") {
        // The Move tool, as after painting a person picks it to arrange.
        key(cx, "escape");
        key(cx, "v");
        outcomes.push(canvas_open("heavy_open", this, cx, fixtures, fixtures.heavy_canvas).await);
    }
    if wanted("heavy_drag") {
        outcomes.push(canvas_drag("heavy_drag", this, cx, fixtures.heavy_layer).await);
    }
    if wanted("heavy_zoom") {
        outcomes.push(canvas_zoom("heavy_zoom", this, cx).await);
    }
    if wanted("blend_open") {
        outcomes.push(canvas_open("blend_open", this, cx, fixtures, fixtures.blend_canvas).await);
    }
    if wanted("blend_zoom") {
        outcomes.push(canvas_zoom("blend_zoom", this, cx).await);
    }
    if wanted("poster_open") {
        outcomes.push(canvas_open("poster_open", this, cx, fixtures, fixtures.poster_canvas).await);
    }
    if wanted("poster_drag") {
        outcomes.push(canvas_drag("poster_drag", this, cx, fixtures.poster_layer).await);
    }
    if wanted("poster_zoom") {
        outcomes.push(zoom_toggle("poster_zoom", this, cx, Workspace::canvas_drawn).await);
    }
    if wanted("poster_brush") {
        outcomes.push(brush_undo("poster_brush", this, cx).await);
        key(cx, "escape");
        key(cx, "v");
    }
    if wanted("poster_zoom_drag") {
        // At 100% the poster renders whole at a capped size; the photo is
        // dragged over it from the middle of the view.
        this.update_in(cx, |ws, window, cx| ws.zoom_any(Some(1.), window, cx))
            .ok();
        until(this, cx, 20_000, Workspace::canvas_drawn).await;
        outcomes.push(canvas_drag("poster_zoom_drag", this, cx, fixtures.poster_layer).await);
        this.update_in(cx, |ws, window, cx| ws.zoom_any(None, window, cx))
            .ok();
        until(this, cx, 20_000, Workspace::canvas_drawn).await;
    }
    if wanted("canvas_tools") {
        outcomes.push(super::features::canvas_tools(this, cx, fixtures).await);
    }
    if wanted("image_sliders") {
        outcomes.push(super::features::image_sliders(this, cx, fixtures).await);
    }
    if wanted("image_zoom") {
        outcomes.push(super::image_zoom::image_zoom(this, cx, fixtures).await);
    }
    outcomes
}

/// Clicks the header's zoom six times: from fit to 100% and back, waiting
/// each time for the view to be drawn sharp.
async fn zoom_toggle(
    name: &'static str,
    this: &This,
    cx: &mut AsyncWindowContext,
    drawn: fn(&Workspace) -> bool,
) -> Outcome {
    until(this, cx, 20_000, drawn).await;
    crate::perf::drain();
    let mut settles = Vec::new();
    let mut problem = None;
    for round in 0..6 {
        let zoom = (round % 2 == 0).then_some(1.);
        this.update_in(cx, |ws, window, cx| ws.zoom_any(zoom, window, cx))
            .ok();
        match until(this, cx, 20_000, drawn).await {
            Some(took) => settles.push(("settle", ms(took))),
            None => problem = Some("the view never came back sharp after zooming".into()),
        }
    }
    outcome(name, settles, problem)
}

pub(super) async fn pause(cx: &mut AsyncWindowContext, ms: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

/// Waits until `ready`, returning how long that took, or `None` after `limit_ms`.
pub(super) async fn until(
    this: &This,
    cx: &mut AsyncWindowContext,
    limit_ms: u64,
    ready: impl Fn(&Workspace) -> bool,
) -> Option<Duration> {
    let started = Instant::now();
    loop {
        if this.read_with(cx, |ws, _| ready(ws)).unwrap_or(false) {
            return Some(started.elapsed());
        }
        if started.elapsed() > Duration::from_millis(limit_ms) {
            return None;
        }
        pause(cx, 5).await;
    }
}

pub(super) fn read<R>(
    this: &This,
    cx: &mut AsyncWindowContext,
    f: impl FnOnce(&Workspace) -> R,
) -> Option<R> {
    this.read_with(cx, |ws, _| f(ws)).ok()
}

/// Presses a key through the app's keymap, as the keyboard does.
pub(super) fn key(cx: &mut AsyncWindowContext, spec: &str) {
    let Ok(keystroke) = Keystroke::parse(spec) else {
        return;
    };
    cx.update(|window, cx| {
        window.dispatch_keystroke(keystroke, cx);
    })
    .ok();
}

/// Presses on the canvas at `path(0)`, moves along `path` for `ms` at 60
/// moves a second, and lets go at `path(1)`: the calls the canvas's mouse
/// listeners make.
pub(super) async fn canvas_drag_along(
    this: &This,
    cx: &mut AsyncWindowContext,
    ms: u64,
    path: impl Fn(f32) -> Point<Pixels>,
) {
    let start = path(0.0);
    this.update_in(cx, |ws, window, cx| {
        let event = MouseDownEvent {
            button: MouseButton::Left,
            position: start,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        };
        ws.canvas_down(&event, window, cx);
    })
    .ok();
    let steps = (ms / 16).max(1);
    for step in 1..=steps {
        let position = path(step as f32 / steps as f32);
        this.update_in(cx, |ws, window, cx| {
            let event = MouseMoveEvent {
                position,
                pressed_button: Some(MouseButton::Left),
                modifiers: Modifiers::default(),
            };
            ws.canvas_move(&event, window, cx);
        })
        .ok();
        pause(cx, 16).await;
    }
    let end = path(1.0);
    this.update_in(cx, |ws, window, cx| ws.canvas_up(end, window, cx))
        .ok();
}

pub(super) async fn click(this: &This, cx: &mut AsyncWindowContext, at: Point<Pixels>) {
    canvas_drag_along(this, cx, 16, |_| at).await;
}

/// Moves a color slider to `value`, as dragging its handle does.
pub(super) fn set_color(this: &This, cx: &mut AsyncWindowContext, index: usize, value: f32) {
    this.update_in(cx, |ws, window, cx| {
        ws.image.color[index].update(cx, |slider, cx| slider.set_value(value, window, cx));
        ws.color_slider_changed(index, window, cx);
    })
    .ok();
}

/// Where a canvas point is in the window.
pub(super) fn on_screen(ws: &Workspace, x: f32, y: f32) -> Point<Pixels> {
    let (rect, scale) = ws.canvas_rect();
    rect.origin + point(px(x * scale), px(y * scale))
}

fn layer_bounds(ws: &Workspace, id: u64) -> Option<([f32; 2], [f32; 2])> {
    ws.canvas.as_ref()?.bounds.get(&id).copied()
}

pub(super) fn outcome(
    name: &'static str,
    extra: Vec<(&'static str, f64)>,
    problem: Option<String>,
) -> Outcome {
    let mut samples: BTreeMap<_, _> = crate::perf::drain();
    for (kind, value) in extra {
        samples.entry(kind).or_default().push(value);
    }
    Outcome {
        name,
        samples,
        problem,
    }
}

pub(super) fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

async fn image_open(
    name: &'static str,
    this: &This,
    cx: &mut AsyncWindowContext,
    fixtures: &Fixtures,
    photo: spectrum_library::AssetId,
) -> Outcome {
    crate::perf::drain();
    let project = fixtures.project;
    this.update_in(cx, |ws, window, cx| {
        ws.enter_project(project, window, cx);
        ws.open_item(Open::Image(photo), window, cx);
    })
    .ok();
    match until(this, cx, 20_000, Workspace::image_idle).await {
        Some(took) => outcome(name, vec![("elapsed", ms(took))], None),
        None => outcome(name, vec![], Some("the photo never showed".into())),
    }
}

/// Drags the Exposure slider back and forth, pausing so each round saves,
/// then sits idle while Spectrum checks for agents' work.
async fn image_scrub(name: &'static str, this: &This, cx: &mut AsyncWindowContext) -> Outcome {
    crate::perf::drain();
    let start = read(this, cx, |ws| ws.image.adjust.exposure).unwrap_or_default();
    // Exposure, back and forth at 60 changes a second, as a drag sends them.
    for drag in 0..4 {
        for step in 0..90 {
            let value = 1.5 * (step as f32 / 90.0 * std::f32::consts::TAU * 1.5).sin();
            set_color(this, cx, 0, start + value + drag as f32 * 0.01);
            pause(cx, 16).await;
        }
        pause(cx, if drag < 3 { 700 } else { 3000 }).await;
    }
    let settled = until(this, cx, 10_000, Workspace::image_idle).await;
    let exposure = read(this, cx, |ws| ws.image.adjust.exposure).unwrap_or_default();
    let problem = if settled.is_none() {
        Some("the photo never caught up with the edits".into())
    } else if exposure == start {
        Some("dragging the Exposure slider did not change the photo".into())
    } else {
        None
    };
    outcome(name, vec![], problem)
}

async fn canvas_open(
    name: &'static str,
    this: &This,
    cx: &mut AsyncWindowContext,
    fixtures: &Fixtures,
    canvas: spectrum_library::AssetId,
) -> Outcome {
    crate::perf::drain();
    let project = fixtures.project;
    this.update_in(cx, |ws, window, cx| {
        ws.enter_project(project, window, cx);
        ws.open_item(Open::Canvas(canvas), window, cx);
    })
    .ok();
    match until(this, cx, 30_000, Workspace::canvas_idle).await {
        Some(took) => outcome(name, vec![("elapsed", ms(took))], None),
        None => outcome(
            name,
            vec![],
            Some("the canvas never finished drawing".into()),
        ),
    }
}

/// Drags the photo layer across the canvas and waits for it to settle.
async fn canvas_drag(
    name: &'static str,
    this: &This,
    cx: &mut AsyncWindowContext,
    id: u64,
) -> Outcome {
    crate::perf::drain();
    let Some((start, before)) = read(this, cx, |ws| {
        let (min, max) = layer_bounds(ws, id)?;
        let x = ws.canvas.as_ref()?.doc.layer(id).ok()?.transform.x;
        // Near the layer's corner, or zoomed in with that off screen, the
        // middle of the part of the layer in view, wherever the window's
        // size puts it.
        let corner = on_screen(ws, min[0] + 60.0, min[1] + 60.0);
        let view = *ws.image.image_bounds.borrow();
        let layer =
            Bounds::from_corners(on_screen(ws, min[0], min[1]), on_screen(ws, max[0], max[1]));
        let seen = view.intersect(&layer);
        let start = if view.contains(&corner) {
            corner
        } else if seen.size.width > px(0.) && seen.size.height > px(0.) {
            seen.center()
        } else {
            return None;
        };
        Some((start, x))
    })
    .flatten() else {
        return outcome(
            name,
            vec![],
            Some("the photo layer is not on the canvas".into()),
        );
    };
    // Picked in the layers panel first, as a person picks a layer under
    // others before dragging it.
    this.update_in(cx, |ws, window, cx| {
        if let Some(canvas) = &mut ws.canvas {
            canvas.selected = Some(id);
        }
        ws.sync_layer_controls(window, cx);
    })
    .ok();
    // As long as a hand takes to reach the canvas from the panel.
    pause(cx, 250).await;
    canvas_drag_along(this, cx, 1000, |t| start + point(px(160. * t), px(90. * t))).await;
    let settled = until(this, cx, 20_000, Workspace::canvas_drawn).await;
    let saved = until(this, cx, 5_000, Workspace::canvas_idle).await;
    let after = read(this, cx, |ws| {
        ws.canvas
            .as_ref()
            .and_then(|c| c.doc.layer(id).ok().map(|l| l.transform.x))
    })
    .flatten();
    let problem = match (settled, after) {
        (None, _) => Some("the canvas never settled after the drag".into()),
        _ if saved.is_none() => Some("the drag was not saved within 5 s".into()),
        (_, Some(x)) if x == before => Some("dragging did not move the photo".into()),
        _ => None,
    };
    let extra = settled
        .map(|took| vec![("settle", ms(took))])
        .unwrap_or_default();
    outcome(name, extra, problem)
}

/// Zooms in and out step by step, waiting for the canvas each time.
async fn canvas_zoom(name: &'static str, this: &This, cx: &mut AsyncWindowContext) -> Outcome {
    crate::perf::drain();
    let mut settles = Vec::new();
    let mut problem = None;
    let zoom = |ws: &Workspace| ws.canvas.as_ref().map(|c| c.zoom);
    let before = read(this, cx, zoom).flatten();
    let steps = ["secondary-="; 5].into_iter().chain(["secondary--"; 5]);
    for spec in steps {
        key(cx, spec);
        pause(cx, 30).await;
        match until(this, cx, 20_000, Workspace::canvas_drawn).await {
            Some(took) => settles.push(("settle", ms(took))),
            None => problem = Some("the canvas never settled after zooming".into()),
        }
    }
    // Zooming back out returns to the starting zoom, so check it moved at all
    // by zooming in once more.
    key(cx, "secondary-=");
    pause(cx, 30).await;
    if read(this, cx, zoom).flatten() == before {
        problem = Some("the zoom keys did not zoom".into());
    }
    until(this, cx, 20_000, Workspace::canvas_idle).await;
    outcome(name, settles, problem)
}

/// Selects the text layer and drags its size slider.
async fn text_size(this: &This, cx: &mut AsyncWindowContext, fixtures: &Fixtures) -> Outcome {
    let id = fixtures.text_layer;
    let Some(at) = read(this, cx, |ws| {
        let (min, max) = layer_bounds(ws, id)?;
        Some(on_screen(
            ws,
            (min[0] + max[0]) / 2.0,
            (min[1] + max[1]) / 2.0,
        ))
    })
    .flatten() else {
        return outcome(
            "text_size",
            vec![],
            Some("the text layer is not on the canvas".into()),
        );
    };
    click(this, cx, at).await;
    until(this, cx, 10_000, Workspace::canvas_idle).await;
    crate::perf::drain();
    for step in 0..40 {
        let size = 140.0 + step as f32 * 3.0;
        this.update_in(cx, |ws, window, cx| {
            ws.canvas_ui
                .text_size
                .update(cx, |slider, cx| slider.set_value(size, window, cx));
            ws.update_text(None, window, cx);
        })
        .ok();
        pause(cx, 16).await;
    }
    let settled = until(this, cx, 20_000, Workspace::canvas_drawn).await;
    let saved = until(this, cx, 5_000, Workspace::canvas_idle).await;
    let size = read(this, cx, |ws| {
        let canvas = ws.canvas.as_ref()?;
        match &canvas.doc.layer(id).ok()?.kind {
            spectrum_canvas::LayerKind::Text { font_size, .. } => Some(*font_size),
            _ => None,
        }
    })
    .flatten();
    let problem = match (settled, size) {
        (None, _) => Some("the canvas never settled after resizing text".into()),
        _ if saved.is_none() => Some("the text size was not saved within 5 s".into()),
        (_, Some(size)) if size < 200.0 => Some("the size slider did not change the text".into()),
        (_, None) => Some("the text layer was not selected".into()),
        _ => None,
    };
    let extra = settled
        .map(|took| vec![("settle", ms(took))])
        .unwrap_or_default();
    outcome("text_size", extra, problem)
}

/// Paints a stroke and undoes it, twice.
async fn brush_undo(name: &'static str, this: &This, cx: &mut AsyncWindowContext) -> Outcome {
    crate::perf::drain();
    key(cx, "escape");
    key(cx, "b");
    pause(cx, 50).await;
    let brush = read(this, cx, |ws| {
        ws.canvas
            .as_ref()
            .is_some_and(|c| c.tool == crate::tools::Tool::Brush)
    })
    .unwrap_or(false);
    if !brush {
        return outcome(
            name,
            vec![],
            Some("pressing B did not choose the Brush".into()),
        );
    }
    let mut settles = Vec::new();
    let mut problem = None;
    for round in 0..2 {
        let y = 760.0 + round as f32 * 60.0;
        let Some((from, to)) = read(this, cx, |ws| {
            (on_screen(ws, 200.0, y), on_screen(ws, 1400.0, y + 80.0))
        }) else {
            break;
        };
        canvas_drag_along(this, cx, 800, |t| from + (to - from) * t).await;
        for step in [None, Some("secondary-z")] {
            if let Some(spec) = step {
                key(cx, spec);
                pause(cx, 30).await;
            }
            match until(this, cx, 20_000, Workspace::canvas_drawn).await {
                Some(took) => settles.push(("settle", ms(took))),
                None => problem = Some("the canvas never settled after painting or undo".into()),
            }
            if until(this, cx, 5_000, Workspace::canvas_idle)
                .await
                .is_none()
            {
                problem = Some("painting or undo was not saved within 5 s".into());
            }
        }
    }
    outcome(name, settles, problem)
}
