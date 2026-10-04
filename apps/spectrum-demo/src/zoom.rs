//! Zooming and panning the canvas. The scroll wheel or a two-finger drag
//! zooms about the pointer; a sideways scroll, ⌘-scroll, Space-drag, or a
//! middle-button drag pans; the header's control and ⌘=, ⌘−, ⌘0, and ⌘⌥0
//! zoom in, out, to fit, and to 100%. Layers keep their images while the
//! zoom changes and render sharp at the new size once it settles.
use crate::{ZoomActual, ZoomFit, ZoomIn, ZoomOut, workspace::Workspace};
use gpui::{prelude::*, *};
use gpui_component::{
    IconName, Sizable,
    button::{Button, ButtonVariants},
};
use std::time::{Duration, Instant};

const MIN_ZOOM: f32 = 0.02;
const MAX_ZOOM: f32 = 32.;
/// The zooms ⌘= and ⌘− step through, as in Photoshop.
const STEPS: [f32; 19] = [
    0.0625, 0.0833, 0.125, 0.1667, 0.25, 0.3333, 0.5, 0.6667, 1., 2., 3., 4., 5., 6., 7., 8., 12.,
    16., 32.,
];
/// How long after the last zoom change the canvas renders at the new size.
pub const SETTLE: Duration = Duration::from_millis(250);
/// Room around a fitted canvas, in screen pixels.
const FIT_MARGIN: f32 = 16.;
/// The canvas stays at least this far on screen when panned.
const KEEP: f32 = 80.;

impl Workspace {
    /// The scale that fits the whole canvas in its area.
    pub fn fit_scale(&self) -> f32 {
        let area = *self.image_bounds.borrow();
        let Some(canvas) = &self.canvas else {
            return 1.;
        };
        let (w, h) = (
            canvas.doc.width.max(1) as f32,
            canvas.doc.height.max(1) as f32,
        );
        // A margin keeps the selected layer's outline and handles in view
        // when it reaches the canvas's edge.
        let margin = 2. * FIT_MARGIN;
        ((f32::from(area.size.width) - margin) / w)
            .min((f32::from(area.size.height) - margin) / h)
            .max(0.001)
    }

    /// The pan that keeps part of a canvas `size` wide on screen.
    fn clamp_pan(&self, pan: (f32, f32), size: (f32, f32)) -> (f32, f32) {
        let area = self.image_bounds.borrow().size;
        let reach = |area: Pixels, size: f32| ((f32::from(area) + size) / 2. - KEEP).max(0.);
        let (rx, ry) = (reach(area.width, size.0), reach(area.height, size.1));
        (pan.0.clamp(-rx, rx), pan.1.clamp(-ry, ry))
    }

    /// Zooms to `zoom`, or to fit for `None`, keeping the canvas point under
    /// `about` (the middle of the view by default) where it is.
    pub fn zoom_to(
        &mut self,
        zoom: Option<f32>,
        about: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let area = *self.image_bounds.borrow();
        let (rect, scale) = self.canvas_rect();
        let Some(canvas) = &self.canvas else {
            return;
        };
        let (w, h) = (canvas.doc.width as f32, canvas.doc.height as f32);
        let placed = zoom.map(|zoom| {
            let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
            let about = about.unwrap_or(area.center());
            let point = (
                f32::from(about.x - rect.origin.x) / scale,
                f32::from(about.y - rect.origin.y) / scale,
            );
            // Where the canvas's corner goes so `point` stays under `about`,
            // as an offset from where it sits centered.
            let pan = (
                f32::from(about.x)
                    - point.0 * zoom
                    - f32::from(area.origin.x)
                    - (f32::from(area.size.width) - w * zoom) / 2.,
                f32::from(about.y)
                    - point.1 * zoom
                    - f32::from(area.origin.y)
                    - (f32::from(area.size.height) - h * zoom) / 2.,
            );
            (zoom, self.clamp_pan(pan, (w * zoom, h * zoom)))
        });
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        match placed {
            Some((zoom, pan)) => {
                canvas.zoom = Some(zoom);
                canvas.pan = pan;
            }
            None => {
                canvas.zoom = None;
                canvas.pan = (0., 0.);
            }
        }
        self.settle_zoom(window, cx);
    }

    /// Renders sharp at the new size once the zoom stops changing.
    fn settle_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(canvas) = &mut self.canvas {
            canvas.zoomed_at = Some(Instant::now());
        }
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(SETTLE + Duration::from_millis(20))
                .await;
            this.update_in(cx, |this, window, cx| {
                this.fit_canvas_resolution(window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Steps to the next zoom in (`1`) or out (`-1`).
    pub fn zoom_step(&mut self, direction: i32, window: &mut Window, cx: &mut Context<Self>) {
        let (_, now) = self.canvas_rect();
        let next = if direction > 0 {
            STEPS.iter().copied().find(|step| *step > now * 1.01)
        } else {
            STEPS.iter().rev().copied().find(|step| *step < now * 0.99)
        };
        if let Some(zoom) = next {
            self.zoom_to(Some(zoom), None, window, cx);
        }
    }

    /// Moves the canvas by a screen distance.
    pub fn pan_by(&mut self, dx: f32, dy: f32, window: &mut Window, cx: &mut Context<Self>) {
        let (_, scale) = self.canvas_rect();
        let Some(canvas) = &self.canvas else {
            return;
        };
        let size = (
            canvas.doc.width as f32 * scale,
            canvas.doc.height as f32 * scale,
        );
        let pan = self.clamp_pan((canvas.pan.0 + dx, canvas.pan.1 + dy), size);
        if let Some(canvas) = &mut self.canvas {
            // Panning a fitted canvas holds its scale.
            canvas.zoom = Some(scale);
            canvas.pan = pan;
        }
        let _ = window;
        cx.notify();
    }

    /// The scroll wheel and two-finger drags: up and down zoom about the
    /// pointer; sideways, or with ⌘ held, they pan.
    pub fn canvas_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A wheel notch is a line or a few; a trackpad gives pixels.
        let (dx, dy, factor) = match event.delta {
            ScrollDelta::Pixels(delta) => {
                let dy = f32::from(delta.y);
                (f32::from(delta.x), dy, (dy * 0.004).exp())
            }
            ScrollDelta::Lines(lines) => (lines.x * 40., lines.y * 40., 1.1_f32.powf(lines.y)),
        };
        cx.stop_propagation();
        if event.modifiers.secondary() || dx.abs() > dy.abs() {
            return self.pan_by(dx, dy, window, cx);
        }
        if dy == 0. {
            return;
        }
        let (_, scale) = self.canvas_rect();
        let zoom = scale * factor.clamp(0.5, 2.);
        self.zoom_to(Some(zoom), Some(event.position), window, cx);
    }

    /// Starts a pan drag from `position` when Space is held or `always`.
    pub fn start_pan(&mut self, position: Point<Pixels>, always: bool) -> bool {
        let Some(canvas) = &mut self.canvas else {
            return false;
        };
        if !(always || canvas.space_held) {
            return false;
        }
        canvas.panning = Some((position, canvas.pan));
        true
    }

    /// Follows a pan drag; false if none is going.
    pub fn pan_move(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((start, pan)) = self.canvas.as_ref().and_then(|c| c.panning) else {
            return false;
        };
        let moved = position - start;
        if let Some(canvas) = &mut self.canvas {
            canvas.pan = pan;
        }
        let (_, scale) = self.canvas_rect();
        if let Some(canvas) = &mut self.canvas {
            canvas.zoom = Some(scale);
        }
        self.pan_by(f32::from(moved.x), f32::from(moved.y), window, cx);
        true
    }

    /// Ends a pan drag; false if none was going.
    pub fn end_pan(&mut self, cx: &mut Context<Self>) -> bool {
        let ended = self
            .canvas
            .as_mut()
            .and_then(|canvas| canvas.panning.take())
            .is_some();
        if ended {
            cx.notify();
        }
        ended
    }

    /// Space held over the canvas turns drags into pans.
    pub fn hold_space(&mut self, held: bool, cx: &mut Context<Self>) {
        if let Some(canvas) = &mut self.canvas
            && canvas.space_held != held
        {
            canvas.space_held = held;
            cx.notify();
        }
    }

    /// The header's zoom: out, the zoom (click for 100%, again to fit), in.
    pub fn zoom_control(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (_, zoom) = self.canvas_rect();
        let actual = (zoom - 1.).abs() < 0.001;
        div()
            .flex()
            .items_center()
            .child(
                Button::new("zoom-out")
                    .ghost()
                    .small()
                    .icon(IconName::Minus)
                    .tooltip("Zoom out (⌘−)")
                    .on_click(cx.listener(|this, _, window, cx| this.zoom_step(-1, window, cx))),
            )
            .child(
                Button::new("zoom-level")
                    .ghost()
                    .small()
                    .w(px(56.))
                    .label(format!("{:.0}%", zoom * 100.))
                    .tooltip(if actual {
                        "Fit the canvas (⌘0)"
                    } else {
                        "Show at 100% (⌘⌥0)"
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let zoom = (!actual).then_some(1.);
                        this.zoom_to(zoom, None, window, cx)
                    })),
            )
            .child(
                Button::new("zoom-in")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip("Zoom in (⌘=)")
                    .on_click(cx.listener(|this, _, window, cx| this.zoom_step(1, window, cx))),
            )
    }

    /// The zoom shortcuts, on the workspace's root.
    pub fn zoom_actions(&self, root: Div, cx: &mut Context<Self>) -> Div {
        root.on_action(cx.listener(|this, _: &ZoomIn, window, cx| this.zoom_step(1, window, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| this.zoom_step(-1, window, cx)))
            .on_action(
                cx.listener(|this, _: &ZoomFit, window, cx| this.zoom_to(None, None, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ZoomActual, window, cx| {
                this.zoom_to(Some(1.), None, window, cx)
            }))
    }
}
