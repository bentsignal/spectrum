//! The canvas in the main area: click a layer to select it, drag to move it.
use crate::{canvas_state::LayerDrag, theme::*, tools::Tool, workspace::Workspace};
use gpui::{prelude::*, *};
use gpui_component::input::{self, Input};
use prism_core::{Command, LayerKind, Transform};

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

    /// Bounds while resizing: the grabbed corner stays put and the layer
    /// scales evenly from the opposite side, growing as the pointer moves out
    /// and shrinking as it moves in.
    fn resized(
        min: [f32; 2],
        max: [f32; 2],
        corner: (bool, bool),
        start: (f32, f32),
        now: (f32, f32),
    ) -> ([f32; 2], [f32; 2], f32) {
        let (width, height) = ((max[0] - min[0]).max(1.), (max[1] - min[1]).max(1.));
        let outward = |right: bool| if right { 1. } else { -1. };
        let fx = (width + outward(corner.0) * (now.0 - start.0)) / width;
        let fy = (height + outward(corner.1) * (now.1 - start.1)) / height;
        let factor = if (fx - 1.).abs() > (fy - 1.).abs() {
            fx
        } else {
            fy
        }
        .max(0.02);
        let (w, h) = (width * factor, height * factor);
        let left = if corner.0 { max[0] - w } else { min[0] };
        let top = if corner.1 { max[1] - h } else { min[1] };
        ([left, top], [left + w, top + h], factor)
    }

    fn canvas_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let point = self.to_canvas(event.position);
        if let Some(canvas) = self.canvas.as_mut().filter(|c| c.tool != Tool::Move) {
            canvas.creating = Some((point, point));
            return cx.notify();
        }
        let corner = self.corner_at(event.position);
        let hit = corner.map(|(id, _)| id).or_else(|| self.layer_at(point));
        if event.click_count == 2
            && hit.is_some()
            && hit == self.canvas.as_ref().and_then(|c| c.selected)
        {
            return self.edit_text(window, cx);
        }
        self.stop_editing_text(window, cx);
        if let Some(id) = hit {
            self.prepare_split(id, window, cx);
        }
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
        if let Some((_, now)) = &mut canvas.creating {
            *now = point;
            return cx.notify();
        }
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
        if let Some(canvas) = &mut self.canvas
            && let Some((start, end)) = canvas.creating.take()
        {
            let tool = canvas.tool;
            return self.create_with_tool(tool, start, end, window, cx);
        }
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
        let corner_bounds = drag.corner.zip(canvas.bounds.get(&drag.id).copied());
        if let Some(canvas) = &mut self.canvas {
            canvas.settling = true;
        }
        if let Some((corner, (min, max))) = corner_bounds {
            let (new_min, new_max, factor) = Self::resized(min, max, corner, drag.start, drag.now);
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
                if let Some(layer) = canvas.doc.layers.iter_mut().find(|l| l.id == drag.id) {
                    layer.transform = transform;
                }
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

    /// Command+E or a double-click: edit the selected text layer on the canvas.
    pub fn edit_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layer) = self.selected_layer() else {
            return;
        };
        if !matches!(layer.kind, LayerKind::Text { .. }) {
            return;
        }
        let id = layer.id;
        if let Some(canvas) = &mut self.canvas {
            canvas.editing = Some(id);
            canvas.drag = None;
        }
        self.sync_layer_controls(window, cx);
        // After the click that started editing has focused the canvas.
        cx.defer_in(window, |this, window, cx| {
            this.text_input
                .update(cx, |state, cx| state.focus(window, cx));
        });
        cx.notify();
    }

    /// Ends on-canvas text editing; keyboard shortcuts go back to the canvas.
    pub fn stop_editing_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .canvas
            .as_mut()
            .and_then(|c| c.editing.take())
            .is_some()
        {
            self.focus_handle.focus(window);
            cx.notify();
        }
    }

    /// Arrow keys: nudge the selected layer by one pixel, or ten with Shift.
    pub fn nudge(&mut self, dx: f32, dy: f32, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.canvas.as_ref().and_then(|c| c.selected) {
            self.prepare_split(id, window, cx);
            if let Some(canvas) = &mut self.canvas {
                canvas.settling = true;
            }
            self.move_layer_by(id, dx, dy, window, cx);
        }
    }

    pub fn canvas_main(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Whole device pixels, so a dragged layer stays as sharp as the canvas.
        let pixel = window.scale_factor();
        let snap = move |value: f32| px((value * pixel).round() / pixel);
        let bounds_slot = self.image_bounds.clone();
        let area = *self.image_bounds.borrow();
        let (rect, scale) = self.canvas_rect();
        let offset = rect.origin - area.origin;
        let image = self.canvas.as_ref().and_then(|c| c.image.clone());
        // Where a layer sits now in canvas space, following any drag.
        let current = |canvas: &crate::canvas_state::CanvasState, id: u64| {
            let (mut min, mut max) = *canvas.bounds.get(&id)?;
            match canvas.drag.filter(|d| d.id == id) {
                Some(LayerDrag {
                    corner: Some(corner),
                    start,
                    now,
                    ..
                }) => {
                    (min, max, _) = Self::resized(min, max, corner, start, now);
                }
                Some(d) => {
                    let (dx, dy) = (d.now.0 - d.start.0, d.now.1 - d.start.1);
                    min = [min[0] + dx, min[1] + dy];
                    max = [max[0] + dx, max[1] + dy];
                }
                None => {}
            }
            Some((min, max))
        };
        let outline = self.canvas.as_ref().and_then(|canvas| {
            let (min, max) = current(canvas, canvas.selected?)?;
            Some(Bounds::from_corners(
                offset + point(px(min[0] * scale), px(min[1] * scale)),
                offset + point(px(max[0] * scale), px(max[1] * scale)),
            ))
        });
        // While a layer moves, draw the rest of the canvas and that layer
        // separately, mapping the layer's rendered bounds onto where it is now.
        let composite = self.canvas.as_ref().and_then(|canvas| {
            let split = canvas.split.as_ref()?;
            let moving = canvas.drag.is_some_and(|d| d.id == split.layer) || canvas.settling;
            let [below, alone, above] = split.ready().filter(|_| moving)?;
            let (min, _) = current(canvas, split.layer)?;
            let (_, max) = current(canvas, split.layer)?;
            // The layer's render covers its bounds when the split was made,
            // rounded up to whole pixels; map that onto its bounds now.
            let (base_min, base_max) = split.base;
            let factor = (max[0] - min[0]) / (base_max[0] - base_min[0]).max(0.001);
            let covered = (
                (base_max[0] - base_min[0]).max(1.).ceil() * factor * scale,
                (base_max[1] - base_min[1]).max(1.).ceil() * factor * scale,
            );
            let origin = point(snap(min[0] * scale), snap(min[1] * scale));
            Some((
                [below, alone, above],
                origin,
                size(px(covered.0), px(covered.1)),
            ))
        });
        let image = if composite.is_some() { None } else { image };
        let loading = image.is_none() && composite.is_none();
        // The box or circle being drawn.
        let drawing = self.canvas.as_ref().and_then(|c| {
            let ((ax, ay), (bx, by)) = c.creating?;
            (c.tool != Tool::Text).then(|| {
                let at = |x: f32, y: f32| offset + point(px(x * scale), px(y * scale));
                (
                    Bounds::from_corners(at(ax.min(bx), ay.min(by)), at(ax.max(bx), ay.max(by))),
                    c.tool == Tool::Circle,
                )
            })
        });
        // The text being edited, in a field attached under the layer.
        let editor = self
            .canvas
            .as_ref()
            .and_then(|c| c.editing)
            .zip(outline)
            .map(|(_, b)| {
                div()
                    .absolute()
                    .left(b.origin.x)
                    .top(b.origin.y + b.size.height + px(10.))
                    .w(b.size.width.max(px(260.)))
                    .rounded_md()
                    .shadow_lg()
                    .child(Input::new(&self.text_input))
                    .on_action(cx.listener(|this, _: &input::Escape, window, cx| {
                        this.stop_editing_text(window, cx)
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            });
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
                .children(image.map(|image| {
                    img(image)
                        .absolute()
                        .left(offset.x)
                        .top(offset.y)
                        .w(rect.size.width)
                        .h(rect.size.height)
                        .object_fit(ObjectFit::Fill)
                }))
                .children(composite.map(|([below, alone, above], origin, size)| {
                    let whole = |image| {
                        img(image)
                            .absolute()
                            .left(offset.x)
                            .top(offset.y)
                            .w(rect.size.width)
                            .h(rect.size.height)
                            .object_fit(ObjectFit::Fill)
                    };
                    div()
                        .absolute()
                        .size_full()
                        .child(whole(below))
                        // Clipped to the canvas, as the finished render will be.
                        .child(
                            div()
                                .absolute()
                                .left(offset.x)
                                .top(offset.y)
                                .w(rect.size.width)
                                .h(rect.size.height)
                                .overflow_hidden()
                                .child(
                                    img(alone)
                                        .absolute()
                                        .left(origin.x)
                                        .top(origin.y)
                                        .w(size.width)
                                        .h(size.height)
                                        .object_fit(ObjectFit::Fill),
                                ),
                        )
                        .child(whole(above))
                }))
                .children(editor)
                .children(drawing.map(|(b, round)| {
                    div()
                        .absolute()
                        .left(b.origin.x)
                        .top(b.origin.y)
                        .w(b.size.width)
                        .h(b.size.height)
                        .border_1()
                        .border_color(hsla(0., 0., 1., 0.9))
                        .when(round, |el| el.rounded_full())
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
