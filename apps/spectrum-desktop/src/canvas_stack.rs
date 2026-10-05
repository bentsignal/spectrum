//! The canvas drawn layer by layer: where each layer is now (following any
//! drag), the layer images in stacking order with a stroke's patch, and
//! which Paint layers are under a point.
use crate::{canvas_state::LayerDrag, workspace::Workspace};
use gpui::{prelude::*, *};
use spectrum_canvas::Transform;
use std::sync::Arc;

/// A layer's image and where it is drawn: id, image, origin, and size.
pub type PlacedLayer = (u64, Arc<RenderImage>, Point<Pixels>, Size<Pixels>);

/// Whether a paint stroke passes within its brush's reach of a canvas point.
pub fn painted_at(
    program: &spectrum_canvas::BrushProgram,
    t: Transform,
    (x, y): (f32, f32),
) -> bool {
    let (x, y) = (
        (x - t.x) / t.scale_x.max(1e-3),
        (y - t.y) / t.scale_y.max(1e-3),
    );
    program.strokes.iter().any(|stroke| {
        if stroke.style.mode == spectrum_canvas::BrushMode::Erase {
            return false;
        }
        let reach = stroke.style.size / 2. + 2.;
        let near = |a: &spectrum_canvas::BrushSample, b: &spectrum_canvas::BrushSample| {
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

impl Workspace {
    /// Layer images in stacking order. A Paint layer with a stroke being
    /// drawn shows its image everywhere but the stroke's patch, and the
    /// patch there; a Paint layer the stroke is making shows just the patch.
    pub fn stacked_layers(
        &self,
        layers: Vec<PlacedLayer>,
        area: Size<Pixels>,
        scale: f32,
    ) -> Vec<AnyElement> {
        let patch = self.canvas.as_ref().and_then(|c| c.stroke_patch.as_ref());
        let place = |image: Arc<RenderImage>, at: Point<Pixels>, shown: Size<Pixels>| {
            img(image)
                .absolute()
                .left(at.x)
                .top(at.y)
                .w(shown.width)
                .h(shown.height)
                .object_fit(ObjectFit::Fill)
        };
        let patch_rect = patch.map(|p| {
            Bounds::from_corners(
                point(px(p.bounds.0[0] * scale), px(p.bounds.0[1] * scale)),
                point(px(p.bounds.1[0] * scale), px(p.bounds.1[1] * scale)),
            )
        });
        let mut elements = Vec::new();
        let mut patched = false;
        for (id, image, at, shown) in layers {
            match (patch, patch_rect) {
                (Some(p), Some(r)) if p.layer == id => {
                    patched = true;
                    // The image around the patch, in four strips.
                    let strips = [
                        Bounds::from_corners(point(px(0.), px(0.)), point(area.width, r.top())),
                        Bounds::from_corners(
                            point(px(0.), r.bottom()),
                            point(area.width, area.height),
                        ),
                        Bounds::from_corners(point(px(0.), r.top()), point(r.left(), r.bottom())),
                        Bounds::from_corners(
                            point(r.right(), r.top()),
                            point(area.width, r.bottom()),
                        ),
                    ];
                    for strip in strips {
                        if strip.size.width <= px(0.) || strip.size.height <= px(0.) {
                            continue;
                        }
                        elements.push(
                            div()
                                .absolute()
                                .left(strip.origin.x)
                                .top(strip.origin.y)
                                .w(strip.size.width)
                                .h(strip.size.height)
                                .overflow_hidden()
                                .child(place(image.clone(), at - strip.origin, shown))
                                .into_any_element(),
                        );
                    }
                    elements.push(place(p.image.clone(), r.origin, r.size).into_any_element());
                }
                _ => elements.push(place(image, at, shown).into_any_element()),
            }
        }
        // A new Paint layer has no image yet; the patch is all of it.
        if let (Some(p), Some(r), false) = (patch, patch_rect, patched) {
            elements.push(place(p.image.clone(), r.origin, r.size).into_any_element());
        }
        elements
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
}
