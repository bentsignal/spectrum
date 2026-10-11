//! Moving a layer inside another (one clipped to the layer below it).
//! The two draw as one image, which would have to render again for every
//! step of the drag; instead, when the drag starts, the rest of the unit
//! and the moving layer render apart, and each frame draws the moving
//! layer where the drag has it, clipped to its holder's box, so it keeps up
//! with the pointer and its outline. The unit renders whole once dropped.
use crate::{
    canvas_split::render_alone,
    layer_cache::{LayerImage, holder_of, unit},
    workspace::Workspace,
};
use gpui::*;

/// The parts of a unit drawn apart while one of its layers moves.
pub struct UnitDrag {
    pub member: u64,
    pub holder: u64,
    /// The unit without the moving layer.
    pub rest: Option<LayerImage>,
    /// The moving layer alone, not clipped.
    pub alone: Option<LayerImage>,
    /// The layer has moved, so the parts show instead of the unit's image.
    pub moved: bool,
    /// The canvas's look version the parts were rendered for.
    version: u64,
}

impl UnitDrag {
    pub fn ready(&self) -> bool {
        self.rest.is_some() && self.alone.is_some()
    }

    fn images(self) -> impl Iterator<Item = std::sync::Arc<RenderImage>> {
        self.rest
            .into_iter()
            .chain(self.alone)
            .map(|part| part.image)
    }
}

impl Workspace {
    /// Whether `member` moves inside its unit with its parts drawn apart.
    pub fn unit_drag_ready(&self, member: u64) -> bool {
        self.canvas.as_ref().is_some_and(|canvas| {
            canvas
                .cache
                .unit_drag
                .as_ref()
                .is_some_and(|drag| drag.member == member && drag.ready())
        })
    }

    /// Starts rendering the parts for moving `member` inside its unit, and
    /// with `moving`, shows them from now on.
    pub fn prepare_unit_drag(
        &mut self,
        member: u64,
        moving: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        let doc = canvas.render_doc();
        let Some(holder) = holder_of(&doc, member).filter(|holder| *holder != member) else {
            return;
        };
        // The drag's moves show the parts a press or selecting the layer
        // rendered; a press renders them afresh only if the canvas changed.
        let version = canvas.look_version();
        if let Some(drag) = canvas
            .cache
            .unit_drag
            .as_mut()
            .filter(|drag| drag.member == member && (moving || drag.version == version))
        {
            drag.moved |= moving;
            return;
        }
        Self::drop_unit_drag(canvas);
        let Some(index) = doc.layers.iter().position(|layer| layer.id == holder) else {
            return;
        };
        let rest: Vec<u64> = unit(&doc, index)
            .into_iter()
            .filter(|id| *id != member)
            .collect();
        let density = canvas
            .cache
            .images
            .get(&holder)
            .map_or(canvas.density, |image| image.density);
        canvas.cache.unit_drag = Some(UnitDrag {
            member,
            holder,
            rest: None,
            alone: None,
            moved: moving,
            version,
        });
        let mut unclipped = doc.clone();
        if let Some(layer) = unclipped.layers.iter_mut().find(|layer| layer.id == member) {
            layer.clip_to_below = false;
        }
        let id = canvas.id;
        for (alone, render, layers) in [(false, doc, rest), (true, unclipped, vec![member])] {
            let root = store.root.clone();
            let task = cx
                .background_executor()
                .spawn(async move { render_alone(&root, &render, &layers, density, None) });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                this.update_in(cx, |this, _, cx| {
                    let Ok(crate::canvas_split::LayerRender {
                        image,
                        bounds,
                        pixel,
                        extent,
                        ..
                    }) = result
                    else {
                        return;
                    };
                    let part = LayerImage {
                        image,
                        pixel,
                        extent,
                        density,
                        bounds,
                        key: String::new(),
                    };
                    let drag = this
                        .canvas
                        .as_mut()
                        .filter(|canvas| canvas.id == id)
                        .and_then(|canvas| canvas.cache.unit_drag.as_mut())
                        .filter(|drag| drag.member == member);
                    let Some(drag) = drag else {
                        crate::retire::drop_later(part.image);
                        return;
                    };
                    let slot = if alone {
                        &mut drag.alone
                    } else {
                        &mut drag.rest
                    };
                    if let Some(old) = slot.replace(part) {
                        crate::retire::drop_later(old.image);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Selecting a layer renders the parts a drag draws it apart with (in
    /// its unit, or over the rest of a canvas drawn as one image), so a drag
    /// right after keeps up from its first frame.
    pub fn prepare_selected_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.canvas.as_ref().and_then(|canvas| canvas.selected) else {
            return;
        };
        if self
            .canvas
            .as_ref()
            .is_some_and(|canvas| canvas.drag.is_none())
        {
            self.prepare_split(id, window, cx);
        }
        let Some(canvas) = &self.canvas else {
            return;
        };
        let prepared = canvas
            .cache
            .unit_drag
            .as_ref()
            .is_some_and(|drag| drag.member == id);
        if canvas.drag.is_none() && !prepared {
            self.prepare_unit_drag(id, false, window, cx);
        }
    }

    /// Frees the parts of a unit drawn apart.
    pub fn drop_unit_drag(canvas: &mut crate::canvas_state::CanvasState) {
        if let Some(drag) = canvas.cache.unit_drag.take() {
            for image in drag.images() {
                crate::retire::drop_later(image);
            }
        }
    }

    /// For the interaction benchmark: how far, in points, the dragged
    /// layer's pixels trail its outline this frame.
    pub fn record_drag_lag(&self, scale: f32, pixel: f32) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let Some(drag) = canvas
            .drag
            .filter(|drag| drag.corner.is_none() && !drag.rotate)
        else {
            return;
        };
        let Some((_, dx, dy)) = self
            .drag_delta()
            .filter(|(_, dx, dy)| *dx != 0. || *dy != 0.)
        else {
            return;
        };
        let shown = canvas.drawn_drag.unwrap_or((0., 0.));
        let behind = (dx - shown.0).hypot(dy - shown.1) * scale;
        let lag = if crate::layer_cache::stackable(&canvas.doc) {
            let apart = holder_of(&canvas.doc, drag.id).is_none_or(|holder| holder == drag.id)
                || self.unit_drag_ready(drag.id);
            if apart { 0. } else { behind }
        } else {
            match canvas
                .split
                .as_ref()
                .filter(|split| split.layer == drag.id && split.ready().is_some())
            {
                // Drawn apart: how far the drawn render is from where the
                // layer is, and how far its size is off.
                Some(split) => self.bounds_now(drag.id).map_or(0., |(min, max)| {
                    let (at, size) =
                        crate::canvas_split::place_split_layer(split, (min, max), scale, pixel);
                    let d = split.density;
                    let expected = [
                        (min[0] + split.pixel[0] / d - split.base.0[0]) * scale,
                        (min[1] + split.pixel[1] / d - split.base.0[1]) * scale,
                    ];
                    let pixels = split.parts[1]
                        .as_ref()
                        .map_or(0., |alone| alone.size(0).width.0 as f32);
                    let width = pixels / d * scale;
                    (f32::from(at.x) - expected[0]).hypot(f32::from(at.y) - expected[1])
                        + (f32::from(size.width) - width).abs()
                }),
                None => behind,
            }
        };
        crate::perf::record_value("drag_lag", f64::from(lag));
    }

    /// For the interaction benchmark: how many points of the stroke being
    /// painted are not on screen yet this frame.
    pub fn record_stroke_lag(&self) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let painting = matches!(
            canvas.tool,
            crate::tools::Tool::Brush | crate::tools::Tool::Eraser
        );
        if painting && canvas.points.len() >= 2 {
            let behind = canvas.points.len().saturating_sub(canvas.stroke_shown);
            crate::perf::record_value("stroke_lag", behind as f64);
        }
    }
}
