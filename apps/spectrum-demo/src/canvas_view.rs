//! The canvas in the main area: click a layer to select it, drag to move it.
use crate::{canvas_state::LayerDrag, theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use prism_core::{Command, Transform};

impl Workspace {
    /// Where the canvas is drawn: fitted and centered, and its scale.
    fn canvas_rect(&self) -> (Bounds<Pixels>, f32) {
        let area = *self.image_bounds.borrow();
        let Some(canvas) = &self.canvas else {
            return (area, 1.);
        };
        let (w, h) = (
            canvas.doc.width.max(1) as f32,
            canvas.doc.height.max(1) as f32,
        );
        let scale = (f32::from(area.size.width) / w).min(f32::from(area.size.height) / h);
        let size = size(px(w * scale), px(h * scale));
        let origin = area.origin
            + point(
                (area.size.width - size.width) / 2.,
                (area.size.height - size.height) / 2.,
            );
        (Bounds::new(origin, size), scale)
    }

    fn to_canvas(&self, position: Point<Pixels>) -> (f32, f32) {
        let (rect, scale) = self.canvas_rect();
        (
            f32::from(position.x - rect.origin.x) / scale,
            f32::from(position.y - rect.origin.y) / scale,
        )
    }

    /// The topmost visible, unlocked layer under a canvas point.
    fn layer_at(&self, (x, y): (f32, f32)) -> Option<u64> {
        let canvas = self.canvas.as_ref()?;
        canvas.doc.layers.iter().rev().find_map(|layer| {
            let (min, max) = canvas.bounds.get(&layer.id)?;
            let inside = x >= min[0] && x <= max[0] && y >= min[1] && y <= max[1];
            (layer.visible && !layer.locked && inside).then_some(layer.id)
        })
    }

    /// The selected layer's corner under the pointer, if any.
    fn corner_at(&self, position: Point<Pixels>) -> Option<(u64, (bool, bool))> {
        let canvas = self.canvas.as_ref()?;
        let id = canvas.selected?;
        let (min, max) = canvas.bounds.get(&id)?;
        let (rect, scale) = self.canvas_rect();
        let near = |x: f32, y: f32| {
            let sx = f32::from(rect.origin.x) + x * scale;
            let sy = f32::from(rect.origin.y) + y * scale;
            (sx - f32::from(position.x)).abs() <= 8. && (sy - f32::from(position.y)).abs() <= 8.
        };
        [(false, false), (true, false), (false, true), (true, true)]
            .into_iter()
            .find(|(r, b)| {
                near(
                    if *r { max[0] } else { min[0] },
                    if *b { max[1] } else { min[1] },
                )
            })
            .map(|corner| (id, corner))
    }

    /// Bounds while resizing: scaled evenly from the opposite corner.
    fn resized(
        min: [f32; 2],
        max: [f32; 2],
        corner: (bool, bool),
        now: (f32, f32),
    ) -> ([f32; 2], [f32; 2], f32) {
        let anchor = [
            if corner.0 { min[0] } else { max[0] },
            if corner.1 { min[1] } else { max[1] },
        ];
        let width = (max[0] - min[0]).max(1.);
        let factor = ((now.0 - anchor[0]).abs() / width).max(0.02);
        let (w, h) = (width * factor, (max[1] - min[1]) * factor);
        let left = if corner.0 { anchor[0] } else { anchor[0] - w };
        let top = if corner.1 { anchor[1] } else { anchor[1] - h };
        ([left, top], [left + w, top + h], factor)
    }

    fn canvas_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let point = self.to_canvas(event.position);
        let corner = self.corner_at(event.position);
        let hit = corner.map(|(id, _)| id).or_else(|| self.layer_at(point));
        if let Some(canvas) = &mut self.canvas {
            canvas.selected = hit;
            canvas.drag = hit.map(|id| LayerDrag {
                id,
                start: point,
                now: point,
                corner: corner.map(|(_, c)| c),
            });
        }
        self.sync_layer_controls(window, cx);
        cx.notify();
    }

    fn canvas_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let point = self.to_canvas(event.position);
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let Some(drag) = &mut canvas.drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            canvas.drag = None;
            return;
        }
        drag.now = point;
        cx.notify();
    }

    fn canvas_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.canvas.as_mut().and_then(|c| c.drag.take()) else {
            return;
        };
        let (dx, dy) = (drag.now.0 - drag.start.0, drag.now.1 - drag.start.1);
        if dx.abs() < 0.5 && dy.abs() < 0.5 {
            cx.notify();
            return;
        }
        let Some(canvas) = &self.canvas else {
            return;
        };
        let Some(layer) = canvas.doc.layers.iter().find(|l| l.id == drag.id) else {
            return;
        };
        let t = layer.transform;
        if let (Some(corner), Some((min, max))) =
            (drag.corner, canvas.bounds.get(&drag.id).copied())
        {
            let (new_min, new_max, factor) = Self::resized(min, max, corner, drag.now);
            // The layer's top-left sits `min - x` from its origin; scale that too.
            let transform = Transform {
                x: new_min[0] - (min[0] - t.x) * factor,
                y: new_min[1] - (min[1] - t.y) * factor,
                scale_x: t.scale_x * factor,
                scale_y: t.scale_y * factor,
                ..t
            };
            if let Some(canvas) = &mut self.canvas {
                canvas.bounds.insert(drag.id, (new_min, new_max));
            }
            return self.canvas_commands(
                vec![Command::SetTransform {
                    id: drag.id,
                    transform,
                }],
                window,
                cx,
            );
        }
        let _ = t;
        self.move_layer_by(drag.id, dx, dy, window, cx);
    }

    /// Moves a layer now in the local copy, so repeated moves build on each
    /// other, and queues the engine command; the render catches up after.
    pub fn move_layer_by(
        &mut self,
        id: u64,
        dx: f32,
        dy: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let Some(layer) = canvas.doc.layers.iter_mut().find(|l| l.id == id) else {
            return;
        };
        layer.transform.x += dx;
        layer.transform.y += dy;
        let transform = layer.transform;
        if let Some((min, max)) = canvas.bounds.get_mut(&id) {
            *min = [min[0] + dx, min[1] + dy];
            *max = [max[0] + dx, max[1] + dy];
        }
        self.canvas_commands(vec![Command::SetTransform { id, transform }], window, cx);
    }

    /// Arrow keys: nudge the selected layer by one pixel, or ten with Shift.
    pub fn nudge(&mut self, dx: f32, dy: f32, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.canvas.as_ref().and_then(|c| c.selected) {
            self.move_layer_by(id, dx, dy, window, cx);
        }
    }

    pub fn canvas_main(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds_slot = self.image_bounds.clone();
        let area = *self.image_bounds.borrow();
        let (rect, scale) = self.canvas_rect();
        let offset = rect.origin - area.origin;
        let render = self.canvas.as_ref().and_then(|c| c.render.clone());
        let outline = self.canvas.as_ref().and_then(|canvas| {
            let id = canvas.selected?;
            let (mut min, mut max) = *canvas.bounds.get(&id)?;
            match canvas.drag.filter(|d| d.id == id) {
                Some(LayerDrag {
                    corner: Some(corner),
                    now,
                    ..
                }) => {
                    (min, max, _) = Self::resized(min, max, corner, now);
                }
                Some(d) => {
                    let (dx, dy) = (d.now.0 - d.start.0, d.now.1 - d.start.1);
                    min = [min[0] + dx, min[1] + dy];
                    max = [max[0] + dx, max[1] + dy];
                }
                None => {}
            }
            Some(Bounds::from_corners(
                offset + point(px(min[0] * scale), px(min[1] * scale)),
                offset + point(px(max[0] * scale), px(max[1] * scale)),
            ))
        });
        let loading = render.is_none();
        div().size_full().p_8().pb(px(40.)).child(
            div()
                .id("canvas-area")
                .relative()
                .size_full()
                .overflow_hidden()
                .child(
                    canvas(
                        move |bounds, _, _| *bounds_slot.borrow_mut() = bounds,
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .when(loading, |el| {
                    el.flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(rgb(FAINT))
                        .child("Rendering…")
                })
                .children(render.map(|path| {
                    img(path)
                        .absolute()
                        .left(offset.x)
                        .top(offset.y)
                        .w(rect.size.width)
                        .h(rect.size.height)
                        .object_fit(ObjectFit::Fill)
                }))
                .children(outline.map(|b| {
                    div()
                        .absolute()
                        .left(b.origin.x - px(1.))
                        .top(b.origin.y - px(1.))
                        .w(b.size.width + px(2.))
                        .h(b.size.height + px(2.))
                        .border_1()
                        .border_color(hsla(0., 0., 1., 0.9))
                }))
                .children(outline.into_iter().flat_map(|b| {
                    [b.origin, b.top_right(), b.bottom_left(), b.bottom_right()].map(|corner| {
                        div()
                            .absolute()
                            .left(corner.x - px(4.5))
                            .top(corner.y - px(4.5))
                            .size(px(9.))
                            .rounded_sm()
                            .bg(rgb(0xf4f4f4))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                    })
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event, window, cx| this.canvas_down(event, window, cx)),
                )
                .on_mouse_move(cx.listener(|this, event, _, cx| this.canvas_move(event, cx)))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.canvas_up(window, cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.canvas_up(window, cx)),
                ),
        )
    }
}
