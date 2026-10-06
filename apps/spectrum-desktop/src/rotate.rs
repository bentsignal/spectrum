//! Rotating the selected layer by its handle above the top edge. The layer
//! re-renders at its new angle as the handle moves (drafts while it moves
//! quickly) and the angle snaps to multiples of 45 degrees nearby, or to
//! 15 degree steps with Shift. Release
//! sends one `SetTransform`, as `spectrum canvas transform`.
use crate::{canvas_state::CanvasState, canvas_state::LayerDrag, workspace::Workspace};
use gpui::*;
use spectrum_canvas::Transform;

/// The handle's distance above the layer, and how close a press must be.
const LIFT: f32 = 26.;
const REACH: f32 = 9.;
const SNAP: f32 = 3.;

impl Workspace {
    /// Where the selected layer's rotate handle is on screen, and the top
    /// center it hangs from.
    pub fn rotate_handle(&self) -> Option<(u64, Point<Pixels>, Point<Pixels>)> {
        let canvas = self.canvas.as_ref()?;
        let id = canvas.selected?;
        let layer = canvas.doc.layer(id).ok()?;
        if layer.locked || canvas.editing.is_some() || canvas.tool != crate::tools::Tool::Move {
            return None;
        }
        let (min, max) = self.bounds_now(id)?;
        let (rect, scale) = self.canvas_rect();
        let top = rect.origin + point(px((min[0] + max[0]) / 2. * scale), px(min[1] * scale));
        Some((id, top - point(px(0.), px(LIFT)), top))
    }

    /// The layer whose rotate handle is under the pointer.
    pub fn rotate_handle_at(&self, position: Point<Pixels>) -> Option<u64> {
        let (id, handle, _) = self.rotate_handle()?;
        let d = position - handle;
        (f32::from(d.x).hypot(f32::from(d.y)) <= REACH).then_some(id)
    }

    /// The transform of a layer being rotated by its handle.
    pub fn rotate_transform(canvas: &CanvasState, drag: LayerDrag) -> Option<Transform> {
        if !drag.rotate {
            return None;
        }
        let (min, max) = *canvas.bounds.get(&drag.id)?;
        let t = canvas.doc.layer(drag.id).ok()?.transform;
        let center = ((min[0] + max[0]) / 2., (min[1] + max[1]) / 2.);
        let angle = |p: (f32, f32)| (p.1 - center.1).atan2(p.0 - center.0).to_degrees();
        let mut rotation = (t.rotation + angle(drag.now) - angle(drag.start)).rem_euclid(360.);
        if drag.constrain {
            // Shift turns in 15 degree steps, as in Photoshop.
            rotation = ((rotation / 15.).round() * 15.).rem_euclid(360.);
        } else {
            let nearest = (rotation / 45.).round() * 45.;
            if (rotation - nearest).abs() <= SNAP {
                rotation = nearest.rem_euclid(360.);
            }
        }
        Some(Transform { rotation, ..t })
    }

    /// Ends a rotation: the new angle is applied and saved.
    pub fn finish_rotate(&mut self, drag: LayerDrag, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let Some(transform) = Self::rotate_transform(canvas, drag) else {
            return;
        };
        if let Some(layer) = canvas.doc.layers.iter_mut().find(|l| l.id == drag.id) {
            layer.transform = transform;
        }
        // The rotated layer's bounds, measured as the engine will.
        if let Ok(layer) = canvas.doc.layer(drag.id)
            && let Ok(geometry) = spectrum_canvas::document_layer_geometry(&canvas.doc, layer)
        {
            canvas.bounds.insert(drag.id, (geometry.min, geometry.max));
        }
        self.canvas_commands(
            vec![spectrum_canvas::Command::SetTransform {
                id: drag.id,
                transform,
            }],
            window,
            cx,
        );
    }

    /// The handle: a dot above the top edge on a short stem.
    pub fn rotate_handle_element(&self) -> Option<AnyElement> {
        let (_, handle, top) = self.rotate_handle()?;
        let origin = self.image.image_bounds.borrow().origin;
        // While turning: how far it has turned and the angle it is at.
        let readout = self.canvas.as_ref().and_then(|canvas| {
            let drag = canvas.drag.filter(|d| d.rotate)?;
            let before = canvas.doc.layer(drag.id).ok()?.transform.rotation;
            let after = Self::rotate_transform(canvas, drag)?.rotation;
            let turned = (after - before + 540.).rem_euclid(360.) - 180.;
            let (rect, scale) = self.canvas_rect();
            let at = rect.origin
                + point(px(drag.now.0 * scale + 18.), px(drag.now.1 * scale + 18.))
                - origin;
            Some(
                div()
                    .absolute()
                    .left(at.x)
                    .top(at.y)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(0x1a1a1a))
                    .border_1()
                    .border_color(rgb(crate::theme::BORDER))
                    .text_xs()
                    .child(format!("{turned:+.0}° · {after:.0}°")),
            )
        });
        let handle_element = Some(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let mut stem = PathBuilder::stroke(px(1.));
                    stem.move_to(handle);
                    stem.line_to(top);
                    if let Ok(path) = stem.build() {
                        window.paint_path(path, hsla(0., 0., 1., 0.9));
                    }
                    let dot = Bounds::centered_at(handle, size(px(10.), px(10.)));
                    window.paint_quad(
                        fill(dot, rgb(0xf4f4f4))
                            .corner_radii(px(5.))
                            .border_widths(px(1.))
                            .border_color(rgb(0x2a2a2a)),
                    );
                },
            )
            .absolute()
            .left(-origin.x)
            .top(-origin.y)
            .size_full()
            .into_any_element(),
        );
        Some(
            div()
                .absolute()
                .size_full()
                .children(handle_element)
                .children(readout)
                .into_any_element(),
        )
    }
}
