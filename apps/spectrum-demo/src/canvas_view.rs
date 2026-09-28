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

    fn canvas_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let point = self.to_canvas(event.position);
        let hit = self.layer_at(point);
        if let Some(canvas) = &mut self.canvas {
            canvas.selected = hit;
            canvas.drag = hit.map(|id| LayerDrag {
                id,
                start: point,
                now: point,
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
        let Some(layer) = self
            .canvas
            .as_ref()
            .and_then(|c| c.doc.layers.iter().find(|l| l.id == drag.id))
        else {
            return;
        };
        let transform = Transform {
            x: layer.transform.x + dx,
            y: layer.transform.y + dy,
            ..layer.transform
        };
        // Move the outline now; the render catches up when the engine finishes.
        if let Some(canvas) = &mut self.canvas
            && let Some((min, max)) = canvas.bounds.get_mut(&drag.id)
        {
            *min = [min[0] + dx, min[1] + dy];
            *max = [max[0] + dx, max[1] + dy];
        }
        self.canvas_commands(
            vec![Command::SetTransform {
                id: drag.id,
                transform,
            }],
            window,
            cx,
        );
    }

    pub fn canvas_main(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds_slot = self.image_bounds.clone();
        let area = *self.image_bounds.borrow();
        let (rect, scale) = self.canvas_rect();
        let offset = rect.origin - area.origin;
        let render = self.canvas.as_ref().and_then(|c| c.render.clone());
        let outline = self.canvas.as_ref().and_then(|canvas| {
            let id = canvas.selected?;
            let (min, max) = canvas.bounds.get(&id)?;
            let (dx, dy) = canvas
                .drag
                .filter(|d| d.id == id)
                .map_or((0., 0.), |d| (d.now.0 - d.start.0, d.now.1 - d.start.1));
            Some(Bounds::from_corners(
                offset + point(px((min[0] + dx) * scale), px((min[1] + dy) * scale)),
                offset + point(px((max[0] + dx) * scale), px((max[1] + dy) * scale)),
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
