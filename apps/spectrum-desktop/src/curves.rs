//! The tone curve editor: click to add a point, drag to move it, and
//! double-click or right-click to remove it. Endpoints stay at the edges.
use crate::{theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use spectrum_image::{CurvePoint, ToneCurve};

pub const CHANNELS: [&str; 4] = ["RGB", "Red", "Green", "Blue"];
const HIT: f32 = 12.;
/// Space around the plot, so points at the edges are easy to grab.
const PAD: f32 = 10.;

fn curve_color(channel: usize) -> Hsla {
    match channel {
        1 => hsla(0.0, 0.75, 0.62, 1.),
        2 => hsla(0.36, 0.6, 0.55, 1.),
        3 => hsla(0.6, 0.75, 0.66, 1.),
        _ => hsla(0., 0., 0.92, 1.),
    }
}

impl Workspace {
    pub fn curve_mut(&mut self) -> &mut ToneCurve {
        let curves = &mut self.image.adjust.curves;
        match self.image.curve_channel {
            1 => &mut curves.red,
            2 => &mut curves.green,
            3 => &mut curves.blue,
            _ => &mut curves.master,
        }
    }

    fn curve(&self) -> &ToneCurve {
        let curves = &self.image.adjust.curves;
        match self.image.curve_channel {
            1 => &curves.red,
            2 => &curves.green,
            3 => &curves.blue,
            _ => &curves.master,
        }
    }

    /// The plot area inside the editor's padding.
    fn curve_plot(&self) -> Bounds<Pixels> {
        let bounds = *self.image.curve_bounds.borrow();
        Bounds::new(
            bounds.origin + point(px(PAD), px(PAD)),
            size(
                bounds.size.width - px(PAD * 2.),
                bounds.size.height - px(PAD * 2.),
            ),
        )
    }

    /// Converts a window position to curve space, 0 to 1 with y up.
    fn curve_point(&self, position: Point<Pixels>) -> (f32, f32) {
        let bounds = self.curve_plot();
        let size = f32::from(bounds.size.width).max(1.);
        let x = f32::from(position.x - bounds.origin.x) / size;
        let y = 1. - f32::from(position.y - bounds.origin.y) / size;
        (x.clamp(0., 1.), y.clamp(0., 1.))
    }

    /// The point under `position`, if any, within a few pixels.
    fn curve_hit(&self, position: Point<Pixels>) -> Option<usize> {
        let bounds = self.curve_plot();
        let size = f32::from(bounds.size.width);
        let distance = |p: &CurvePoint| {
            let px_ = f32::from(bounds.origin.x) + p.x * size;
            let py = f32::from(bounds.origin.y) + (1. - p.y) * size;
            (px_ - f32::from(position.x)).hypot(py - f32::from(position.y))
        };
        // The nearest point within reach, so close neighbors stay pickable.
        self.curve()
            .points
            .iter()
            .enumerate()
            .map(|(index, p)| (index, distance(p)))
            .filter(|(_, d)| *d <= HIT)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    fn curve_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let hit = self.curve_hit(event.position);
        let last = self.curve().points.len().saturating_sub(1);
        let remove = event.button == MouseButton::Right || event.click_count == 2;
        if remove {
            if let Some(index) = hit.filter(|i| *i != 0 && *i != last) {
                self.curve_mut().points.remove(index);
                self.image.curve_drag = None;
                self.schedule_color_edit(window, cx);
            }
            return;
        }
        if let Some(index) = hit {
            self.image.curve_drag = Some(index);
            cx.notify();
            return;
        }
        let (x, y) = self.curve_point(event.position);
        let points = &mut self.curve_mut().points;
        if points.len() >= 32 {
            return;
        }
        let index = points.iter().position(|p| p.x > x).unwrap_or(points.len());
        points.insert(index, CurvePoint { x, y });
        self.image.curve_drag = Some(index);
        self.schedule_color_edit(window, cx);
    }

    fn curve_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.image.curve_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.image.curve_drag = None;
            return;
        }
        let (x, y) = self.curve_point(event.position);
        let points = &mut self.curve_mut().points;
        let last = points.len() - 1;
        let x = match index {
            0 => 0.,
            i if i == last => 1.,
            i => x.clamp(points[i - 1].x + 0.01, points[i + 1].x - 0.01),
        };
        points[index] = CurvePoint { x, y };
        self.schedule_color_edit(window, cx);
    }

    pub fn curve_editor(&self, edge: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let points: Vec<(f32, f32)> = self.curve().points.iter().map(|p| (p.x, p.y)).collect();
        let color = curve_color(self.image.curve_channel);
        let bounds_slot = self.image.curve_bounds.clone();
        let dragging = self.image.curve_drag;
        let view = cx.entity();
        div()
            .id("curve-editor")
            .size(px(edge))
            .rounded_md()
            .bg(rgb(SURFACE))
            .child(
                canvas(
                    move |bounds, _, _| *bounds_slot.borrow_mut() = bounds,
                    move |bounds, _, window, _| {
                        // Follow a drag anywhere in the window, not just over the editor.
                        if dragging.is_some() {
                            let moving = view.clone();
                            window.on_mouse_event(
                                move |event: &MouseMoveEvent, phase, window, cx| {
                                    if phase == DispatchPhase::Bubble {
                                        moving.update(cx, |this, cx| {
                                            this.curve_move(event, window, cx)
                                        });
                                    }
                                },
                            );
                            let released = view.clone();
                            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Bubble {
                                    released.update(cx, |this, cx| {
                                        this.image.curve_drag = None;
                                        cx.notify();
                                    });
                                }
                            });
                        }
                        let o = bounds.origin + point(px(PAD), px(PAD));
                        let s = f32::from(bounds.size.width) - PAD * 2.;
                        let at = |x: f32, y: f32| o + point(px(x * s), px((1. - y) * s));
                        let faint = hsla(0., 0., 1., 0.07);
                        for i in 1..4 {
                            let t = i as f32 / 4.;
                            window.paint_quad(fill(
                                Bounds::new(at(t, 1.), size(px(1.), px(s))),
                                faint,
                            ));
                            window.paint_quad(fill(
                                Bounds::new(at(0., t), size(px(s), px(1.))),
                                faint,
                            ));
                        }
                        let mut diagonal = PathBuilder::stroke(px(1.));
                        diagonal.move_to(at(0., 0.));
                        diagonal.line_to(at(1., 1.));
                        if let Ok(path) = diagonal.build() {
                            window.paint_path(path, hsla(0., 0., 1., 0.12));
                        }
                        let mut curve = PathBuilder::stroke(px(1.5));
                        for (i, (x, y)) in points.iter().enumerate() {
                            if i == 0 {
                                curve.move_to(at(*x, *y));
                            } else {
                                curve.line_to(at(*x, *y));
                            }
                        }
                        if let Ok(path) = curve.build() {
                            window.paint_path(path, color);
                        }
                        for (i, (x, y)) in points.iter().enumerate() {
                            let r = if dragging == Some(i) { 5. } else { 4. };
                            let c = at(*x, *y);
                            window.paint_quad(
                                fill(
                                    Bounds::new(
                                        c - point(px(r), px(r)),
                                        size(px(r * 2.), px(r * 2.)),
                                    ),
                                    color,
                                )
                                .corner_radii(px(r)),
                            );
                        }
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, window, cx| this.curve_down(event, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event, window, cx| this.curve_down(event, window, cx)),
            )
    }
}
