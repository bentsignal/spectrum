//! The canvas in the main area: click a layer to select it, drag to move it.
use crate::{canvas_state::LayerDrag, theme::*, tools::Tool, workspace::Workspace};
use gpui::{prelude::*, *};
use gpui_component::input::{self, Input};
use prism_core::{Command, LayerKind, Transform};
use std::sync::Arc;

/// Whether a paint stroke passes within its brush's reach of a canvas point.
fn painted_at(program: &prism_core::BrushProgram, t: Transform, (x, y): (f32, f32)) -> bool {
    let (x, y) = (
        (x - t.x) / t.scale_x.max(1e-3),
        (y - t.y) / t.scale_y.max(1e-3),
    );
    program.strokes.iter().any(|stroke| {
        if stroke.style.mode == prism_core::BrushMode::Erase {
            return false;
        }
        let reach = stroke.style.size / 2. + 2.;
        let near = |a: &prism_core::BrushSample, b: &prism_core::BrushSample| {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let length = dx * dx + dy * dy;
            let t = if length > 0. {
                (((x - a.x) * dx + (y - a.y) * dy) / length).clamp(0., 1.)
            } else {
                0.
            };
            (a.x + dx * t - x).hypot(a.y + dy * t - y) <= reach
        };
        match stroke.samples.len() {
            0 => false,
            1 => near(&stroke.samples[0], &stroke.samples[0]),
            _ => stroke
                .samples
                .windows(2)
                .any(|pair| near(&pair[0], &pair[1])),
        }
    })
}

/// A move in canvas units, rounded to whole pixels of the canvas render, so
/// the moved layer's pixels match its render exactly when dropped.
pub fn whole_pixels(delta: f32, density: f32) -> f32 {
    (delta * density).round() / density.max(f32::EPSILON)
}

impl Workspace {
    /// Where the canvas is drawn: fitted and centered, and its scale.
    pub fn canvas_rect(&self) -> (Bounds<Pixels>, f32) {
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

    /// The topmost visible, unlocked layer under a canvas point. Paint
    /// layers cover the canvas, so they count only where they have paint.
    fn layer_at(&self, (x, y): (f32, f32)) -> Option<u64> {
        let canvas = self.canvas.as_ref()?;
        canvas.doc.layers.iter().rev().find_map(|layer| {
            let (min, max) = canvas.bounds.get(&layer.id)?;
            let inside = x >= min[0] && x <= max[0] && y >= min[1] && y <= max[1];
            let painted = match &layer.kind {
                LayerKind::Paint { program } => painted_at(program, layer.transform, (x, y)),
                _ => true,
            };
            (layer.visible && !layer.locked && inside && painted).then_some(layer.id)
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

    /// The transform and bounds of a layer being resized from a corner.
    pub fn resize_transform(
        canvas: &crate::canvas_state::CanvasState,
        drag: LayerDrag,
    ) -> Option<(Transform, crate::canvas_split::LayerBounds)> {
        let corner = drag.corner?;
        let (min, max) = *canvas.bounds.get(&drag.id)?;
        let t = canvas
            .doc
            .layers
            .iter()
            .find(|l| l.id == drag.id)?
            .transform;
        let (new_min, new_max, factor) = Self::resized(min, max, corner, drag.start, drag.now);
        // The layer's top-left sits `min - x` from its origin; scale that too.
        let transform = Transform {
            x: new_min[0] - (min[0] - t.x) * factor,
            y: new_min[1] - (min[1] - t.y) * factor,
            scale_x: t.scale_x * factor,
            scale_y: t.scale_y * factor,
            ..t
        };
        Some((transform, (new_min, new_max)))
    }

    /// Where a layer sits now in canvas space, following any drag: moved,
    /// resized from a corner, or (while rotating) where its latest render fell.
    pub fn bounds_now(&self, id: u64) -> Option<crate::canvas_split::LayerBounds> {
        let canvas = self.canvas.as_ref()?;
        let moved = self.drag_delta();
        // A layer only in the stroke being drawn has its render's bounds.
        let (mut min, mut max) = match canvas.bounds.get(&id) {
            Some(bounds) => *bounds,
            None => canvas.cache.images.get(&id)?.bounds,
        };
        if canvas.drag.is_some_and(|d| d.id == id && d.rotate)
            && let Some(cached) = canvas.cache.images.get(&id)
        {
            return Some(cached.bounds);
        }
        match canvas.drag.filter(|d| d.id == id) {
            Some(LayerDrag {
                corner: Some(corner),
                start,
                now,
                ..
            }) => {
                (min, max, _) = Self::resized(min, max, corner, start, now);
            }
            Some(_) => {
                // Pressed but not moved yet: the layer stays where it is.
                let (dx, dy) = moved.map_or((0., 0.), |(_, dx, dy)| (dx, dy));
                min = [min[0] + dx, min[1] + dy];
                max = [max[0] + dx, max[1] + dy];
            }
            None => {}
        }
        Some((min, max))
    }

    fn canvas_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let point = self.to_canvas(event.position);
        // Keys go to the canvas after a click on it (tool keys, Delete).
        if self.canvas.as_ref().is_some_and(|c| c.editing.is_none()) {
            self.focus_handle.focus(window);
        }
        // Selection tools act on the layer under the pointer when none is
        // chosen; the wand always takes the layer it was clicked on.
        let under = self.layer_at(point);
        if let Some(canvas) = self.canvas.as_mut().filter(|c| c.tool != Tool::Move) {
            canvas.select_mode = crate::selection_view::combine_mode(event.modifiers);
            if canvas.tool == Tool::Wand && under.is_some()
                || canvas.tool.selects() && canvas.selected.is_none()
            {
                canvas.selected = under.or(canvas.selected);
            }
            match canvas.tool {
                Tool::Pen => return self.pen_down(point, event.modifiers.shift, window, cx),
                Tool::Wand => return self.magic_wand(point, window, cx),
                Tool::Eyedropper => {
                    return self.pick_canvas_color(point, event.modifiers.alt, window, cx);
                }
                _ => {}
            }
            let tool = canvas.tool;
            if matches!(tool, Tool::Brush | Tool::Eraser) && !self.can_stroke(tool, window, cx) {
                return;
            }
            if let Some(canvas) = &mut self.canvas {
                canvas.creating = Some((point, point));
                canvas.points = vec![point];
            }
            if matches!(tool, Tool::Brush | Tool::Eraser) {
                self.update_live_stroke(tool, window, cx);
            }
            return cx.notify();
        }
        if let Some(id) = self.rotate_handle_at(event.position)
            && let Some(canvas) = &mut self.canvas
        {
            canvas.drag = Some(LayerDrag {
                id,
                start: point,
                now: point,
                corner: None,
                rotate: true,
                constrain: event.modifiers.shift,
            });
            return cx.notify();
        }
        let corner = self.corner_at(event.position);
        if corner.is_none()
            && let Some(guide) = self.guide_at(event.position)
            && let Some(canvas) = &mut self.canvas
        {
            canvas.guide_drag = Some(guide);
            return cx.notify();
        }
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
                rotate: false,
                constrain: false,
            });
        }
        self.sync_layer_controls(window, cx);
        cx.notify();
    }

    fn canvas_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.canvas.as_ref().is_some_and(|c| c.guide_drag.is_some()) {
            return self.move_guide(event.position, cx);
        }
        let point = self.to_canvas(event.position);
        let (_, scale) = self.canvas_rect();
        let hover = self
            .canvas
            .as_ref()
            .filter(|c| c.tool == Tool::Move && c.drag.is_none())
            .and_then(|_| self.guide_at(event.position))
            .map(|g| g.orientation);
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        if canvas.hover_guide != hover {
            canvas.hover_guide = hover;
            cx.notify();
        }
        canvas.pointer = Some(point);
        if canvas.tool == Tool::Pen {
            let pressed = event.pressed_button == Some(MouseButton::Left);
            if pressed {
                return self.pen_drag(point, event.modifiers.shift, cx);
            }
            return cx.notify();
        }
        if matches!(canvas.tool, Tool::Brush | Tool::Eraser) && canvas.creating.is_none() {
            // The brush outline follows the pointer.
            cx.notify();
        }
        if canvas.tool == Tool::Eyedropper {
            self.ensure_pixels(window, cx);
            return cx.notify();
        }
        if let Some((_, now)) = &mut canvas.creating {
            *now = point;
            // Freehand tools keep every point at least a screen pixel apart.
            let freehand = matches!(canvas.tool, Tool::Lasso | Tool::Brush | Tool::Eraser);
            if freehand
                && canvas
                    .points
                    .last()
                    .is_none_or(|l| (l.0 - point.0).hypot(l.1 - point.1) * scale >= 1.)
            {
                canvas.points.push(point);
                let tool = canvas.tool;
                if matches!(tool, Tool::Brush | Tool::Eraser) {
                    self.update_live_stroke(tool, window, cx);
                }
            }
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
        drag.constrain = event.modifiers.shift;
        let drag = *drag;
        // Canvases drawn as one render re-render with the layer where the
        // drag has it.
        if !crate::layer_cache::stackable(&canvas.doc) {
            let shaped = Self::resize_transform(canvas, drag)
                .map(|(transform, _)| transform)
                .or_else(|| Self::rotate_transform(canvas, drag));
            let t = canvas.doc.layer(drag.id).ok().map(|l| l.transform);
            let preview = shaped.or_else(|| {
                let (_, dx, dy) = self.drag_delta()?;
                let t = t?;
                Some(prism_core::Transform {
                    x: t.x + dx,
                    y: t.y + dy,
                    ..t
                })
            });
            if let Some(canvas) = &mut self.canvas {
                canvas.drag_preview = preview.map(|t| (drag.id, t));
                canvas.bump_version();
            }
            self.render_canvas(window, cx);
        }
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let Some(drag) = &mut canvas.drag else {
            return;
        };
        // A resize re-renders the layer at its new size as it goes, so its
        // effects keep their own size instead of stretching with it.
        let in_unit = crate::layer_cache::holder_of(&canvas.doc, drag.id).is_some_and(|holder| {
            holder != drag.id
                || canvas
                    .doc
                    .layers
                    .iter()
                    .position(|l| l.id == holder)
                    .is_some_and(|i| crate::layer_cache::unit(&canvas.doc, i).len() > 1)
        });
        if drag.corner.is_some() || drag.rotate || in_unit {
            self.refresh_layers(window, cx);
        }
        cx.notify();
    }

    fn canvas_up(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if self.canvas.as_ref().is_some_and(|c| c.tool == Tool::Pen) {
            return self.pen_up();
        }
        if self.canvas.as_ref().is_some_and(|c| c.guide_drag.is_some()) {
            self.move_guide(at, cx);
            return self.drop_guide(window, cx);
        }
        let moved = self.drag_delta();
        if let Some(canvas) = &mut self.canvas {
            canvas.drag_preview = None;
        }
        if let Some(canvas) = &mut self.canvas
            && let Some((start, end)) = canvas.creating.take()
        {
            let tool = canvas.tool;
            return self.create_with_tool(tool, start, end, window, cx);
        }
        let Some(drag) = self.canvas.as_mut().and_then(|c| c.drag.take()) else {
            return;
        };
        let Some(canvas) = &self.canvas else {
            return;
        };
        if drag.rotate {
            return self.finish_rotate(drag, window, cx);
        }
        let (dx, dy) = moved.map_or((0., 0.), |(_, dx, dy)| (dx, dy));
        if dx == 0. && dy == 0. && drag.corner.is_none() {
            cx.notify();
            return;
        }
        let resize = Self::resize_transform(canvas, drag);
        if let Some(canvas) = &mut self.canvas {
            canvas.settling = true;
        }
        if let Some((transform, (new_min, new_max))) = resize {
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
        let offset = point(snap(f32::from(offset.x)), snap(f32::from(offset.y)));
        // A whole-canvas render at its own pixel size, so it shows pixel for
        // pixel; stretched to the canvas only until a render for a new size
        // arrives.
        let fit = move |image: &Arc<RenderImage>| {
            let pixels = image.size(0);
            let own = size(
                px(pixels.width.0 as f32 / pixel),
                px(pixels.height.0 as f32 / pixel),
            );
            if (own.width - rect.size.width).abs() <= px(2. / pixel) {
                own
            } else {
                rect.size
            }
        };
        let image = self.canvas.as_ref().and_then(|c| c.image.clone());
        // Where a layer sits now in canvas space, following any drag.
        let current = |_: &crate::canvas_state::CanvasState, id: u64| self.bounds_now(id);
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
            let (min, max) = current(canvas, split.layer)?;
            // Map the layer's render from where it was rendered to where the
            // layer is now: moved whole, or scaled while resizing.
            let (base_min, base_max) = split.base;
            let factor = (max[0] - min[0]) / (base_max[0] - base_min[0]).max(0.001);
            let d = split.density;
            let (origin, layer_size) = if (factor - 1.).abs() < 0.0005 {
                // Moving: shift the render by whole pixels and show it pixel
                // for pixel, exactly as the full render will place it.
                let shift = [
                    ((min[0] - base_min[0]) * d).round(),
                    ((min[1] - base_min[1]) * d).round(),
                ];
                let pixels = alone.size(0);
                (
                    point(
                        px((split.pixel[0] + shift[0]) / pixel),
                        px((split.pixel[1] + shift[1]) / pixel),
                    ),
                    size(
                        px(pixels.width.0 as f32 / pixel),
                        px(pixels.height.0 as f32 / pixel),
                    ),
                )
            } else {
                // Resizing: scale the render about the layer's bounds.
                let at = [
                    min[0] + (split.pixel[0] / d - base_min[0]) * factor,
                    min[1] + (split.pixel[1] / d - base_min[1]) * factor,
                ];
                (
                    point(snap(at[0] * scale), snap(at[1] * scale)),
                    size(
                        px(split.extent[0] * factor * scale),
                        px(split.extent[1] * factor * scale),
                    ),
                )
            };
            Some(([below, alone, above], origin, layer_size))
        });
        // Stackable canvases: the background, then every layer from its own
        // image, placed by whole device pixels where the layer is now.
        let stack = self
            .canvas
            .as_ref()
            .filter(|c| c.loaded)
            .and_then(|canvas| {
                if !crate::layer_cache::stackable(&canvas.doc) {
                    return None;
                }
                let device = scale * pixel;
                let shown = canvas.shown();
                let layers = shown
                    .layers
                    .iter()
                    .enumerate()
                    // Layers inside another are drawn in its image.
                    .filter(|(index, layer)| {
                        layer.visible && !crate::layer_cache::inside(shown, *index)
                    })
                    .map(|(_, layer)| layer)
                    .filter_map(|layer| {
                        let cached = canvas.cache.images.get(&layer.id)?;
                        let (min, max) = current(canvas, layer.id)?;
                        let (base_min, base_max) = cached.bounds;
                        let factor = (max[0] - min[0]) / (base_max[0] - base_min[0]).max(0.001);
                        let exact = (factor - 1.).abs() < 0.001
                            && (cached.density - device).abs() <= device * 0.002;
                        let placed = if exact {
                            let shift = [
                                ((min[0] - base_min[0]) * cached.density).round(),
                                ((min[1] - base_min[1]) * cached.density).round(),
                            ];
                            let pixels = cached.image.size(0);
                            (
                                point(
                                    px((cached.pixel[0] + shift[0]) / pixel),
                                    px((cached.pixel[1] + shift[1]) / pixel),
                                ),
                                size(
                                    px(pixels.width.0 as f32 / pixel),
                                    px(pixels.height.0 as f32 / pixel),
                                ),
                            )
                        } else {
                            // Resizing, or rendered for another scale: map the
                            // render's canvas area onto the layer's bounds now.
                            let at = [
                                min[0] + (cached.pixel[0] / cached.density - base_min[0]) * factor,
                                min[1] + (cached.pixel[1] / cached.density - base_min[1]) * factor,
                            ];
                            (
                                point(px(at[0] * scale), px(at[1] * scale)),
                                size(
                                    px(cached.extent[0] * factor * scale),
                                    px(cached.extent[1] * factor * scale),
                                ),
                            )
                        };
                        Some((cached.image.clone(), placed.0, placed.1))
                    })
                    .collect::<Vec<_>>();
                let [r, g, b, a] = canvas.doc.background;
                Some((rgba(u32::from_be_bytes([r, g, b, a])), layers))
            });
        let image = if composite.is_some() || stack.is_some() {
            None
        } else {
            image
        };
        let composite = if stack.is_some() { None } else { composite };
        let loading = image.is_none() && composite.is_none() && stack.is_none();
        // The gradient being dragged out, as a line from start to end.
        let gradient_line = self.canvas.as_ref().and_then(|c| {
            let ((ax, ay), (bx, by)) = c.creating?;
            (c.tool == Tool::Gradient).then(|| {
                let at = |x: f32, y: f32| offset + point(px(x * scale), px(y * scale));
                (at(ax, ay), at(bx, by))
            })
        });
        // The box or circle being drawn.
        let drawing = self.canvas.as_ref().and_then(|c| {
            let ((ax, ay), (bx, by)) = c.creating?;
            matches!(c.tool, Tool::Box | Tool::Circle).then(|| {
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
                .when(
                    self.canvas.as_ref().is_some_and(|c| c.tool != Tool::Move),
                    |el| el.cursor_crosshair(),
                )
                .map(
                    |el| match self.canvas.as_ref().and_then(|c| c.hover_guide) {
                        Some(prism_core::GuideOrientation::Vertical) => el.cursor_col_resize(),
                        Some(prism_core::GuideOrientation::Horizontal) => el.cursor_row_resize(),
                        None => el,
                    },
                )
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
                .children(stack.map(|(background, layers)| {
                    div()
                        .absolute()
                        .left(offset.x)
                        .top(offset.y)
                        .w(rect.size.width)
                        .h(rect.size.height)
                        .overflow_hidden()
                        .bg(background)
                        .children(layers.into_iter().map(|(image, at, shown)| {
                            img(image)
                                .absolute()
                                .left(at.x)
                                .top(at.y)
                                .w(shown.width)
                                .h(shown.height)
                                .object_fit(ObjectFit::Fill)
                        }))
                }))
                .children(image.map(|image| {
                    let shown = fit(&image);
                    img(image)
                        .absolute()
                        .left(offset.x)
                        .top(offset.y)
                        .w(shown.width)
                        .h(shown.height)
                        .object_fit(ObjectFit::Fill)
                }))
                .children(composite.map(|([below, alone, above], origin, size)| {
                    let whole = |image: Arc<RenderImage>| {
                        let shown = fit(&image);
                        img(image)
                            .absolute()
                            .left(offset.x)
                            .top(offset.y)
                            .w(shown.width)
                            .h(shown.height)
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
                .child(self.guide_overlay(offset, scale, rect.size))
                .children(self.selection_overlay(offset, scale))
                .children(self.brush_overlay(offset, scale, cx))
                .children(self.pen_overlay(offset, scale))
                .children(self.crop_overlay(offset, scale))
                .children(self.eyedropper_overlay(offset, scale))
                .children(gradient_line.map(|(from, to)| {
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            let (from, to) = (bounds.origin + from, bounds.origin + to);
                            let mut line = PathBuilder::stroke(px(1.5));
                            line.move_to(from);
                            line.line_to(to);
                            if let Ok(path) = line.build() {
                                window.paint_path(path, hsla(0., 0., 1., 0.9));
                            }
                            for end in [from, to] {
                                let mut dot = PathBuilder::fill();
                                let r = px(3.5);
                                dot.move_to(end + point(r, px(0.)));
                                dot.arc_to(
                                    point(r, r),
                                    px(0.),
                                    false,
                                    true,
                                    end - point(r, px(0.)),
                                );
                                dot.arc_to(
                                    point(r, r),
                                    px(0.),
                                    false,
                                    true,
                                    end + point(r, px(0.)),
                                );
                                if let Ok(path) = dot.build() {
                                    window.paint_path(path, hsla(0., 0., 1., 0.95));
                                }
                            }
                        },
                    )
                    .absolute()
                    .size_full()
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
                .children(self.rotate_handle_element())
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
                .on_mouse_move(
                    cx.listener(|this, event, window, cx| this.canvas_move(event, window, cx)),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseUpEvent, window, cx| {
                        this.canvas_up(event.position, window, cx)
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseUpEvent, window, cx| {
                        this.canvas_up(event.position, window, cx)
                    }),
                ),
        )
    }
}
