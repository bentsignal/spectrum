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
pub const SETTLE: Duration = Duration::from_millis(60);
/// How long a zoom from the keys or the header takes to arrive.
const ANIMATION: Duration = Duration::from_millis(140);

/// A zoom from the keys or the header on its way: it moves smoothly about
/// the point both views share while the canvas renders for where it is
/// going, so it arrives sharp instead of blurring and then snapping.
#[derive(Clone, Copy)]
pub struct ZoomAnim {
    from: (Bounds<Pixels>, f32),
    started: Instant,
}

impl ZoomAnim {
    /// A zoom starting from where something is drawn now.
    pub fn new(from: (Bounds<Pixels>, f32)) -> Self {
        Self {
            from,
            started: Instant::now(),
        }
    }

    /// The canvas's place and scale now, on the way to `target`.
    pub fn at(
        &self,
        target: (Bounds<Pixels>, f32),
        _area: Bounds<Pixels>,
    ) -> (Bounds<Pixels>, f32) {
        let t = (self.started.elapsed().as_secs_f32() / ANIMATION.as_secs_f32()).min(1.);
        let eased = 1. - (1. - t).powi(3);
        let ((from, from_scale), (to, to_scale)) = (self.from, target);
        let scale = from_scale * (to_scale / from_scale).powf(eased);
        let origin = if (from_scale - to_scale).abs() > 1e-4 {
            // The canvas point that stays put, and where it is on screen.
            let anchor = |a: Pixels, b: Pixels| {
                let point = f32::from(b - a) / (from_scale - to_scale);
                f32::from(a) + point * from_scale - point * scale
            };
            point(
                px(anchor(from.origin.x, to.origin.x)),
                px(anchor(from.origin.y, to.origin.y)),
            )
        } else {
            from.origin + (to.origin - from.origin) * eased
        };
        let size = size(
            to.size.width * (scale / to_scale),
            to.size.height * (scale / to_scale),
        );
        (Bounds::new(origin, size), scale)
    }

    pub fn done(&self) -> bool {
        self.started.elapsed() >= ANIMATION
    }
}

/// Room around a fitted canvas, in screen pixels.
const FIT_MARGIN: f32 = 16.;
/// The canvas stays at least this far on screen when panned.
const KEEP: f32 = 80.;

impl Workspace {
    /// The scale that fits the whole canvas in its area.
    pub fn fit_scale(&self) -> f32 {
        let area = *self.image.image_bounds.borrow();
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
        let area = self.image.image_bounds.borrow().size;
        let reach = |area: Pixels, size: f32| ((f32::from(area) + size) / 2. - KEEP).max(0.);
        let (rx, ry) = (reach(area.width, size.0), reach(area.height, size.1));
        (pan.0.clamp(-rx, rx), pan.1.clamp(-ry, ry))
    }

    /// Zooms to `zoom`, or to fit for `None`, about the middle of the view,
    /// moving there smoothly: the keys and the header's zoom.
    pub fn zoom_to(&mut self, zoom: Option<f32>, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.canvas_rect();
        self.place_zoom(zoom, None);
        if let Some(canvas) = &mut self.canvas {
            canvas.zoom_anim = Some(ZoomAnim::new(from));
            canvas.zoomed_at = None;
        }
        // Render for where the zoom is going at once.
        self.fit_canvas_resolution(window, cx);
        self.refresh_layers(window, cx);
        self.refresh_composite_detail(window, cx);
        cx.notify();
    }

    /// Ends a finished zoom animation, rendering what came into view.
    pub fn step_zoom_animation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(anim) = self.canvas.as_ref().and_then(|c| c.zoom_anim) else {
            return;
        };
        if !anim.done() {
            window.request_animation_frame();
            return;
        }
        if let Some(canvas) = &mut self.canvas {
            canvas.zoom_anim = None;
        }
        self.refresh_layers(window, cx);
        self.refresh_composite_detail(window, cx);
    }

    /// Zooms to `zoom` at once, keeping the canvas point under `about` where
    /// it is: a scroll or two-finger gesture.
    fn zoom_about(
        &mut self,
        zoom: f32,
        about: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(canvas) = &mut self.canvas {
            canvas.zoom_anim = None;
        }
        self.place_zoom(Some(zoom), Some(about));
        self.settle_zoom(window, cx);
    }

    /// Sets the zoom and pan for `zoom` (fit for `None`) about `about`.
    fn place_zoom(&mut self, zoom: Option<f32>, about: Option<Point<Pixels>>) {
        let area = *self.image.image_bounds.borrow();
        let (rect, scale) = self.canvas_rect_target();
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
                this.refresh_details(window, cx);
                this.refresh_composite_detail(window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Steps to the next zoom in (`1`) or out (`-1`).
    pub fn zoom_step(&mut self, direction: i32, window: &mut Window, cx: &mut Context<Self>) {
        let (_, now) = self.canvas_rect_target();
        let next = if direction > 0 {
            STEPS.iter().copied().find(|step| *step > now * 1.01)
        } else {
            STEPS.iter().rev().copied().find(|step| *step < now * 0.99)
        };
        if let Some(zoom) = next {
            self.zoom_to(Some(zoom), window, cx);
        }
    }

    /// Moves the canvas by a screen distance.
    pub fn pan_by(&mut self, dx: f32, dy: f32, window: &mut Window, cx: &mut Context<Self>) {
        let (_, scale) = self.canvas_rect_target();
        if let Some(canvas) = &mut self.canvas {
            canvas.zoom_anim = None;
        }
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
        self.refresh_details_after_pan(window, cx);
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
        cx.stop_propagation();
        let Some(scroll) = self.gestures.read(event) else {
            return;
        };
        match scroll.gesture {
            crate::gesture::Gesture::Pan => self.pan_by(scroll.dx, scroll.dy, window, cx),
            crate::gesture::Gesture::Zoom => {
                let (_, scale) = self.canvas_rect_target();
                self.zoom_about(scale * scroll.factor, event.position, window, cx);
            }
        }
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

    /// Whether the canvas shows whole, fitted to its area.
    fn canvas_fitted(&self) -> bool {
        let (_, zoom) = self.canvas_rect_target();
        self.canvas.as_ref().is_some_and(|canvas| {
            canvas.zoom.is_none()
                || ((zoom - self.fit_scale()).abs() < 0.001
                    && canvas.pan.0.abs() < 0.5
                    && canvas.pan.1.abs() < 0.5)
        })
    }

    /// The header's zoom for the open canvas or image: out, the zoom (click
    /// to fit, and from fit to 100%), in.
    pub fn zoom_control(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (zoom, fitted, what) = if self.image_zooms() {
            let zoom = self
                .image
                .view
                .zoom
                .or_else(|| self.image_fit_zoom(window))
                .unwrap_or(1.);
            (zoom, self.image.view.zoom.is_none(), "image")
        } else {
            let (_, zoom) = self.canvas_rect_target();
            (zoom, self.canvas_fitted(), "canvas")
        };
        div()
            .flex()
            .items_center()
            .child(
                Button::new("zoom-out")
                    .ghost()
                    .small()
                    .icon(IconName::Minus)
                    .tooltip("Zoom out (⌘−)")
                    .on_click(cx.listener(|this, _, window, cx| this.zoom_by(-1, window, cx))),
            )
            .child(
                Button::new("zoom-level")
                    .ghost()
                    .small()
                    .w(px(56.))
                    .label(format!("{:.0}%", zoom * 100.))
                    .tooltip(if fitted {
                        "Show at 100% (⌘⌥0)"
                    } else if what == "image" {
                        "Fit the image (⌘0)"
                    } else {
                        "Fit the canvas (⌘0)"
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let zoom = fitted.then_some(1.);
                        this.zoom_any(zoom, window, cx)
                    })),
            )
            .child(
                Button::new("zoom-in")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip("Zoom in (⌘=)")
                    .on_click(cx.listener(|this, _, window, cx| this.zoom_by(1, window, cx))),
            )
    }

    /// Steps the open image's or canvas's zoom.
    pub(crate) fn zoom_by(&mut self, direction: i32, window: &mut Window, cx: &mut Context<Self>) {
        if self.image_zooms() {
            self.image_zoom_step(direction, window, cx);
        } else {
            self.zoom_step(direction, window, cx);
        }
    }

    /// Zooms the open image or canvas to `zoom`, or to fit for `None`.
    pub(crate) fn zoom_any(
        &mut self,
        zoom: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.image_zooms() {
            self.image_zoom_to(zoom, None, true, window, cx);
        } else {
            self.zoom_to(zoom, window, cx);
        }
    }

    /// The zoom shortcuts, on the workspace's root.
    pub fn zoom_actions(&self, root: Div, cx: &mut Context<Self>) -> Div {
        // In the video editor these zoom the timeline.
        root.on_action(
            cx.listener(|this, _: &ZoomIn, window, cx| match this.video {
                Some(_) => this.zoom_timeline(1, cx),
                None => this.zoom_by(1, window, cx),
            }),
        )
        .on_action(
            cx.listener(|this, _: &ZoomOut, window, cx| match this.video {
                Some(_) => this.zoom_timeline(-1, cx),
                None => this.zoom_by(-1, window, cx),
            }),
        )
        .on_action(
            cx.listener(|this, _: &ZoomFit, window, cx| match this.video {
                Some(_) => this.zoom_timeline(0, cx),
                None => this.zoom_any(None, window, cx),
            }),
        )
        .on_action(
            cx.listener(|this, _: &ZoomActual, window, cx| this.zoom_any(Some(1.), window, cx)),
        )
    }
}
